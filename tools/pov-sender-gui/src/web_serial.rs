use std::cell::RefCell;

use futures_channel::oneshot;
use futures_util::{
    future::{Either, select},
    pin_mut,
};
use gloo_timers::future::TimeoutFuture;
use js_sys::{Array, Function, Promise, Reflect, Uint8Array};
use pov_proto::{
    bridge::{BridgeControlRequest, BridgeControlResponse},
    transfer::{AdcDevice, AdcSample, SpokeCommand, SpokeResponse, parse_packet},
};
use pov_sender_core::{
    DeviceStorageStats, DownloadKind, EspNowDelivery, EspNowPeer, PolarEncodeOptions,
    SerialLinkConfig, Transport, chunk_download_payload, encode_bridge_frame,
    encode_command_packet, encode_command_packet_with_transfer_id, encode_image_bytes,
    encode_video_bytes, max_chunk_payload,
};
use wasm_bindgen_futures::JsFuture;
use web_sys::wasm_bindgen::closure::Closure;
use web_sys::wasm_bindgen::{JsCast, JsValue};

pub type WebSerialPort = web_sys::SerialPort;

const LIST_PEERS_RETRY_DELAY_MS: u32 = 250;
const RX_BUF: usize = 2048;
const RESPONSE_TIMEOUT_MS: u32 = 5_000;

#[derive(Clone, Debug)]
pub struct SelectedWebFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

thread_local! {
    static WEB_PORTS: RefCell<Vec<WebSerialPort>> = const { RefCell::new(Vec::new()) };
}

pub fn cached_ports() -> Vec<WebSerialPort> {
    WEB_PORTS.with(|ports| ports.borrow().clone())
}

pub async fn list_port_labels() -> Result<Vec<String>, String> {
    let serial = browser_serial()?;
    let values = JsFuture::from(serial.get_ports()).await.map_err(js_err)?;

    let ports: Vec<WebSerialPort> = Array::from(&values)
        .iter()
        .filter_map(|value| value.dyn_into::<WebSerialPort>().ok())
        .collect();

    WEB_PORTS.with(|cached| {
        *cached.borrow_mut() = ports.clone();
    });

    Ok(ports
        .iter()
        .enumerate()
        .map(|(index, _)| format!("Web Serial Port {}", index + 1))
        .collect())
}

pub async fn request_port_and_list_labels() -> Result<Vec<String>, String> {
    let serial = browser_serial()?;
    let _ = JsFuture::from(serial.request_port())
        .await
        .map_err(js_err)?;

    list_port_labels().await
}

pub async fn send_command_over_web_serial(
    port: WebSerialPort,
    config: SerialLinkConfig,
    command: SpokeCommand,
) -> Result<String, String> {
    let packet = encode_command_packet(command).map_err(|e| e.to_string())?;
    let stats = send_packets_over_web_serial(&port, &config, &[packet]).await?;

    Ok(format!(
        "Command sent: {} packet(s), {} transmission(s)",
        stats.packet_count, stats.total_transmissions
    ))
}

pub async fn list_esp_now_peers_over_web_serial(
    port: WebSerialPort,
    baud: u32,
) -> Result<Vec<EspNowPeer>, String> {
    match list_esp_now_peers_once(&port, baud).await {
        Ok(peers) => Ok(peers),
        Err(first_err) => {
            TimeoutFuture::new(LIST_PEERS_RETRY_DELAY_MS).await;
            list_esp_now_peers_once(&port, baud).await.map_err(|retry_err| {
                format!(
                    "Failed to list ESP-NOW peers after retry (delay {} ms). First error: {}. Retry error: {}",
                    LIST_PEERS_RETRY_DELAY_MS, first_err, retry_err
                )
            })
        }
    }
}

pub async fn request_storage_stats_over_web_serial(
    port: WebSerialPort,
    config: SerialLinkConfig,
) -> Result<DeviceStorageStats, String> {
    let target_peer = require_stateful_espnow_target(&config, "Storage stats request")?;
    let transfer_id = 0x53544154usize;
    let packet =
        encode_command_packet_with_transfer_id(transfer_id, SpokeCommand::RequestStorageStats)
            .map_err(|e| e.to_string())?;

    let mut session = WebSerialSession::open(port, config.baud).await?;
    let result = async {
        session.discard_buffered_frames()?;
        write_packet_frame(&session, &config, &packet).await?;

        loop {
            let response = session
                .read_bridge_control_response("storage stats")
                .await?;
            match response {
                BridgeControlResponse::EspNowPeers(_) => {}
                BridgeControlResponse::EspNowInboundPacket { src, payload } => {
                    if src != target_peer {
                        continue;
                    }

                    let Ok(packet) = parse_packet(payload) else {
                        continue;
                    };
                    match packet {
                        pov_proto::transfer::Packet::Response(frame)
                            if frame.transfer_id == transfer_id =>
                        {
                            let SpokeResponse::StorageStats(stats) = frame.response else {
                                continue;
                            };
                            return Ok(DeviceStorageStats {
                                total_bytes: stats.total_bytes,
                                used_bytes: stats.used_bytes,
                                free_bytes: stats.free_bytes,
                                image_count: stats.image_count,
                                active_image_id: stats.active_image_id,
                            });
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    .await;

    session.finish(result).await
}

pub async fn request_adc_sample_over_web_serial(
    port: WebSerialPort,
    config: SerialLinkConfig,
    device: AdcDevice,
) -> Result<AdcSample, String> {
    let target_peer = require_stateful_espnow_target(&config, "ADC sample request")?;
    let transfer_id = 0x4144_4353usize;
    let packet = encode_command_packet_with_transfer_id(
        transfer_id,
        SpokeCommand::RequestAdcSample { device },
    )
    .map_err(|e| e.to_string())?;

    let mut session = WebSerialSession::open(port, config.baud).await?;
    let result = async {
        session.discard_buffered_frames()?;
        write_packet_frame(&session, &config, &packet).await?;

        loop {
            let response = session.read_bridge_control_response("ADC sample").await?;
            match response {
                BridgeControlResponse::EspNowPeers(_) => {}
                BridgeControlResponse::EspNowInboundPacket { src, payload } => {
                    if src != target_peer {
                        continue;
                    }

                    let Ok(packet) = parse_packet(payload) else {
                        continue;
                    };
                    match packet {
                        pov_proto::transfer::Packet::Response(frame)
                            if frame.transfer_id == transfer_id =>
                        {
                            let SpokeResponse::AdcSample(sample) = frame.response else {
                                continue;
                            };
                            return Ok(sample);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    .await;

    session.finish(result).await
}

pub async fn send_image_file_over_web_serial(
    port: WebSerialPort,
    config: SerialLinkConfig,
    file: SelectedWebFile,
    polar: Option<PolarEncodeOptions>,
    gif_max_fps: Option<u16>,
) -> Result<String, String> {
    let is_gif = file.name.to_ascii_lowercase().ends_with(".gif");
    let payload = if is_gif {
        encode_video_bytes(&file.name, &file.bytes, polar, gif_max_fps)
            .map_err(|e| e.to_string())?
    } else {
        encode_image_bytes(&file.name, &file.bytes, polar).map_err(|e| e.to_string())?
    };

    let kind = if is_gif {
        DownloadKind::Video
    } else {
        DownloadKind::DisplayImage
    };

    let packets = chunk_download_payload(&payload, kind, max_chunk_payload(config.transport))
        .map_err(|e| e.to_string())?;
    let stats = send_packets_over_web_serial(&port, &config, &packets).await?;

    if is_gif {
        Ok(format!(
            "Video sent from GIF: {} packet(s), {} transmission(s)",
            stats.packet_count, stats.total_transmissions
        ))
    } else {
        Ok(format!(
            "Image sent: {} packet(s), {} transmission(s)",
            stats.packet_count, stats.total_transmissions
        ))
    }
}

pub async fn send_ota_file_over_web_serial(
    port: WebSerialPort,
    config: SerialLinkConfig,
    file: SelectedWebFile,
) -> Result<String, String> {
    let packets = chunk_download_payload(
        &file.bytes,
        DownloadKind::OtaImage,
        max_chunk_payload(config.transport),
    )
    .map_err(|e| e.to_string())?;

    let stats = send_packets_over_web_serial(&port, &config, &packets).await?;

    Ok(format!(
        "OTA sent: {} packet(s), {} transmission(s)",
        stats.packet_count, stats.total_transmissions
    ))
}

pub async fn pick_file(accept: &str) -> Result<SelectedWebFile, String> {
    let window = web_sys::window().ok_or_else(|| "window is not available".to_string())?;
    let document = window
        .document()
        .ok_or_else(|| "document is not available".to_string())?;

    let input: web_sys::HtmlInputElement = document
        .create_element("input")
        .map_err(js_err)?
        .dyn_into()
        .map_err(|_| "Failed to create file input element".to_string())?;
    input.set_type("file");
    input.set_accept(accept);

    let (tx, rx) = oneshot::channel::<Result<web_sys::File, String>>();
    let input_for_handler = input.clone();
    let mut tx = Some(tx);
    let on_change = Closure::wrap(Box::new(move |_event: web_sys::Event| {
        let result = input_for_handler
            .files()
            .and_then(|files| files.get(0))
            .ok_or_else(|| "No file selected".to_string());
        if let Some(sender) = tx.take() {
            let _ = sender.send(result);
        }
    }) as Box<dyn FnMut(_)>);

    input.set_onchange(Some(on_change.as_ref().unchecked_ref()));
    on_change.forget();
    input.click();

    let file = rx.await.map_err(|_| "File picker canceled".to_string())??;

    let buffer = JsFuture::from(file.array_buffer()).await.map_err(js_err)?;
    let data = Uint8Array::new(&buffer);
    let mut bytes = vec![0u8; data.length() as usize];
    data.copy_to(&mut bytes);

    Ok(SelectedWebFile {
        name: file.name(),
        bytes,
    })
}

fn browser_serial() -> Result<web_sys::Serial, String> {
    let window = web_sys::window().ok_or_else(|| "window is not available".to_string())?;
    let navigator = window.navigator();

    let has_serial = Reflect::has(&navigator, &JsValue::from_str("serial")).map_err(js_err)?;
    if !has_serial {
        return Err(
            "Web Serial API is not available. Desktop Chromium over HTTPS or localhost is required. Chrome on Android only supports Web Serial over Bluetooth RFCOMM and does not support wired USB serial yet."
                .to_string(),
        );
    }

    Ok(navigator.serial())
}

async fn open_port(port: &WebSerialPort, baud: u32) -> Result<(), String> {
    if port_is_open(port)? {
        return Ok(());
    }

    let options = web_sys::SerialOptions::new(baud);
    Reflect::set(
        options.as_ref(),
        &JsValue::from_str("bufferSize"),
        &JsValue::from_f64(RX_BUF as f64),
    )
    .map_err(js_err)?;
    Reflect::set(
        options.as_ref(),
        &JsValue::from_str("dataBits"),
        &JsValue::from_f64(8.0),
    )
    .map_err(js_err)?;
    Reflect::set(
        options.as_ref(),
        &JsValue::from_str("stopBits"),
        &JsValue::from_f64(1.0),
    )
    .map_err(js_err)?;
    Reflect::set(
        options.as_ref(),
        &JsValue::from_str("parity"),
        &JsValue::from_str("none"),
    )
    .map_err(js_err)?;
    Reflect::set(
        options.as_ref(),
        &JsValue::from_str("flowControl"),
        &JsValue::from_str("none"),
    )
    .map_err(js_err)?;
    JsFuture::from(port.open(&options)).await.map_err(js_err)?;
    set_port_signals(port).await;
    Ok(())
}

fn port_is_open(port: &WebSerialPort) -> Result<bool, String> {
    let readable = Reflect::get(port.as_ref(), &JsValue::from_str("readable")).map_err(js_err)?;
    let writable = Reflect::get(port.as_ref(), &JsValue::from_str("writable")).map_err(js_err)?;
    Ok(!readable.is_null()
        && !readable.is_undefined()
        && !writable.is_null()
        && !writable.is_undefined())
}

async fn set_port_signals(port: &WebSerialPort) {
    let Ok(set_signals) = Reflect::get(port.as_ref(), &JsValue::from_str("setSignals")) else {
        return;
    };
    if set_signals.is_undefined() || set_signals.is_null() {
        return;
    }

    let Ok(function) = set_signals.dyn_into::<Function>() else {
        return;
    };

    let options = js_sys::Object::new();
    let _ = Reflect::set(
        options.as_ref(),
        &JsValue::from_str("dataTerminalReady"),
        &JsValue::TRUE,
    );
    let _ = Reflect::set(
        options.as_ref(),
        &JsValue::from_str("requestToSend"),
        &JsValue::TRUE,
    );

    let Ok(promise) = function.call1(port.as_ref(), options.as_ref()) else {
        return;
    };
    let Ok(promise) = promise.dyn_into::<Promise>() else {
        return;
    };
    let _ = JsFuture::from(promise).await;
}

async fn list_esp_now_peers_once(
    port: &WebSerialPort,
    baud: u32,
) -> Result<Vec<EspNowPeer>, String> {
    let mut session = WebSerialSession::open(port.clone(), baud).await?;
    let result = async {
        session.discard_buffered_frames()?;
        let request =
            pov_proto::bridge::BridgeFrame::ControlRequest(BridgeControlRequest::ListEspNowPeers);
        let frame = postcard::to_stdvec_cobs(&request)
            .map_err(|e| format!("Failed to encode peer-list request: {e}"))?;
        session.write_bytes(&frame).await?;

        loop {
            let response = session.read_bridge_control_response("list peers").await?;
            match response {
                BridgeControlResponse::EspNowPeers(list) => {
                    let count = usize::from(list.count).min(list.peers.len());
                    let peers = list.peers[..count]
                        .iter()
                        .copied()
                        .map(|mac| EspNowPeer { mac })
                        .collect();
                    return Ok(peers);
                }
                BridgeControlResponse::EspNowInboundPacket { .. } => {}
            }
        }
    }
    .await;

    session.finish(result).await
}

async fn write_packet_frame(
    session: &WebSerialSession,
    config: &SerialLinkConfig,
    packet: &[u8],
) -> Result<(), String> {
    let frame = encode_bridge_frame(
        config.transport,
        config.esp_now_delivery,
        config.esp_now_retries,
        packet,
    )
    .map_err(|e| e.to_string())?;
    session.write_bytes(&frame).await?;

    let delay_ms = u32::try_from(config.inter_packet_delay_ms).unwrap_or(u32::MAX);
    if delay_ms > 0 {
        TimeoutFuture::new(delay_ms).await;
    }

    Ok(())
}

fn require_stateful_espnow_target(
    config: &SerialLinkConfig,
    operation: &str,
) -> Result<[u8; 6], String> {
    if config.transport != Transport::Espnow {
        return Err(format!("{operation} requires espnow transport"));
    }

    match config.esp_now_delivery {
        EspNowDelivery::Peer(peer) => Ok(peer),
        EspNowDelivery::Broadcast => Err(format!("{operation} requires stateful peer target")),
    }
}

async fn send_packets_over_web_serial(
    port: &WebSerialPort,
    config: &SerialLinkConfig,
    packets: &[Vec<u8>],
) -> Result<SendStats, String> {
    open_port(port, config.baud).await?;

    let writable = port.writable();
    let writer = writable.get_writer().map_err(js_err)?;
    let repeat = config.repeat.max(1);

    for _ in 0..repeat {
        for packet in packets {
            let bytes = encode_bridge_frame(
                config.transport,
                config.esp_now_delivery,
                config.esp_now_retries,
                packet,
            )
            .map_err(|e| e.to_string())?;
            let chunk = Uint8Array::from(bytes.as_slice());
            JsFuture::from(writer.write_with_chunk(&chunk.into()))
                .await
                .map_err(js_err)?;

            let delay_ms = u32::try_from(config.inter_packet_delay_ms).unwrap_or(u32::MAX);
            if delay_ms > 0 {
                TimeoutFuture::new(delay_ms).await;
            }
        }
    }

    writer.release_lock();

    Ok(SendStats {
        packet_count: packets.len(),
        total_transmissions: packets.len() * repeat,
    })
}

#[derive(Clone, Copy, Debug)]
struct SendStats {
    packet_count: usize,
    total_transmissions: usize,
}

fn js_err(value: JsValue) -> String {
    value.as_string().unwrap_or_else(|| format!("{value:?}"))
}

struct WebSerialSession {
    port: WebSerialPort,
    reader: web_sys::ReadableStreamDefaultReader,
    pending: Vec<u8>,
}

impl WebSerialSession {
    async fn open(port: WebSerialPort, baud: u32) -> Result<Self, String> {
        open_port(&port, baud).await?;
        let reader = port
            .readable()
            .get_reader()
            .dyn_into::<web_sys::ReadableStreamDefaultReader>()
            .map_err(|_| "Failed to acquire readable stream reader".to_string())?;
        Ok(Self {
            port,
            reader,
            pending: Vec::new(),
        })
    }

    async fn finish<T>(self, result: Result<T, String>) -> Result<T, String> {
        let release_result = self.release();
        match (result, release_result) {
            (Ok(value), Ok(())) => Ok(value),
            (Ok(_), Err(release_err)) => Err(release_err),
            (Err(err), Ok(())) => Err(err),
            (Err(err), Err(release_err)) => Err(format!(
                "{}; additionally failed to release Web Serial reader: {}",
                err, release_err
            )),
        }
    }

    fn release(self) -> Result<(), String> {
        self.reader.release_lock();
        Ok(())
    }

    fn discard_buffered_frames(&mut self) -> Result<(), String> {
        while self.take_next_frame()?.is_some() {}
        Ok(())
    }

    async fn write_bytes(&self, bytes: &[u8]) -> Result<(), String> {
        let writer = self.port.writable().get_writer().map_err(js_err)?;
        let chunk = Uint8Array::from(bytes);
        let write_result = JsFuture::from(writer.write_with_chunk(&chunk.into()))
            .await
            .map_err(js_err);
        writer.release_lock();
        write_result.map(|_| ())
    }

    async fn read_bridge_control_response(
        &mut self,
        context: &str,
    ) -> Result<BridgeControlResponse<'static>, String> {
        loop {
            if let Some(frame) = self.take_next_frame()? {
                let leaked = Box::leak(frame.into_boxed_slice());
                match postcard::from_bytes_cobs::<BridgeControlResponse<'_>>(leaked) {
                    Ok(response) => return Ok(response),
                    Err(_) => {
                        // Web Serial reads can begin mid-frame after the port opens.
                        // Discard malformed data and continue scanning for the next delimiter.
                        continue;
                    }
                }
            }

            let chunk = read_chunk_with_timeout(&self.reader, context).await?;
            if chunk.done {
                return Err(format!(
                    "Serial stream closed while waiting for bridge response ({context})"
                ));
            }

            if self.pending.len().saturating_add(chunk.bytes.len()) > RX_BUF {
                return Err(format!("Bridge response exceeded buffer size ({context})"));
            }

            self.pending.extend(chunk.bytes);
        }
    }

    fn take_next_frame(&mut self) -> Result<Option<Vec<u8>>, String> {
        let Some(zero_index) = self.pending.iter().position(|byte| *byte == 0) else {
            return Ok(None);
        };

        let frame = self.pending.drain(..zero_index).collect::<Vec<_>>();
        self.pending.drain(..1);

        if frame.is_empty() {
            return Ok(None);
        }

        if frame.len() > RX_BUF {
            return Err("Bridge response exceeded buffer size".to_string());
        }

        Ok(Some(frame))
    }
}

struct ReadChunk {
    done: bool,
    bytes: Vec<u8>,
}

async fn read_chunk_with_timeout(
    reader: &web_sys::ReadableStreamDefaultReader,
    context: &str,
) -> Result<ReadChunk, String> {
    let read = JsFuture::from(reader.read());
    let timeout = TimeoutFuture::new(RESPONSE_TIMEOUT_MS);
    pin_mut!(read);
    pin_mut!(timeout);

    match select(read, timeout).await {
        Either::Left((result, _)) => {
            let result = result.map_err(js_err)?;
            let done = Reflect::get(&result, &JsValue::from_str("done"))
                .map_err(js_err)?
                .as_bool()
                .unwrap_or(false);
            let value = Reflect::get(&result, &JsValue::from_str("value")).map_err(js_err)?;

            if done || value.is_undefined() || value.is_null() {
                return Ok(ReadChunk {
                    done,
                    bytes: Vec::new(),
                });
            }

            let chunk = Uint8Array::new(&value);
            let mut bytes = vec![0u8; chunk.length() as usize];
            chunk.copy_to(&mut bytes);

            Ok(ReadChunk { done, bytes })
        }
        Either::Right((_, _)) => {
            cancel_reader(reader).await?;
            Err(format!("Timed out waiting for bridge response ({context})"))
        }
    }
}

async fn cancel_reader(reader: &web_sys::ReadableStreamDefaultReader) -> Result<(), String> {
    let cancel = Reflect::get(reader.as_ref(), &JsValue::from_str("cancel")).map_err(js_err)?;
    if cancel.is_undefined() || cancel.is_null() {
        return Ok(());
    }

    let function: Function = cancel
        .dyn_into()
        .map_err(|_| "Readable stream reader does not expose cancel()".to_string())?;
    let promise = function.call0(reader.as_ref()).map_err(js_err)?;
    let promise: Promise = promise
        .dyn_into()
        .map_err(|_| "Readable stream reader cancel() did not return a promise".to_string())?;
    JsFuture::from(promise).await.map_err(js_err)?;
    Ok(())
}
