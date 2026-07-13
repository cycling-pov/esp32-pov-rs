use std::cell::RefCell;

use futures_channel::oneshot;
use gloo_timers::future::TimeoutFuture;
use js_sys::{Array, Reflect, Uint8Array};
use pov_sender_core::{
    DownloadKind, PolarEncodeOptions, SerialLinkConfig, SpokeCommand, chunk_download_payload,
    encode_bridge_frame, encode_command_packet, encode_image_bytes, encode_video_bytes,
    max_chunk_payload,
};
use wasm_bindgen_futures::JsFuture;
use web_sys::wasm_bindgen::closure::Closure;
use web_sys::wasm_bindgen::{JsCast, JsValue};

pub type WebSerialPort = web_sys::SerialPort;

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
    let options = web_sys::SerialOptions::new(baud);
    JsFuture::from(port.open(&options)).await.map_err(js_err)?;
    Ok(())
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
    JsFuture::from(port.close()).await.map_err(js_err)?;

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
