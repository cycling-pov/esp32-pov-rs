use std::cell::RefCell;

use futures_channel::oneshot;
use gloo_timers::future::TimeoutFuture;
use image::AnimationDecoder;
use js_sys::{Array, Reflect, Uint8Array};
use pov_proto::{
    bridge::{BridgeFrame, EspNowTarget, TransportSelector},
    image::{LedCount, RadialCount, encode_polar_rgb888_to_wire, encode_rgb888_to_wire},
    transfer::{ChunkIter, CommandFrame, DownloadKind, Packet, SpokeCommand, encode_packet},
    video,
};
use pov_sender_core::{EspNowDelivery, PolarEncodeOptions, SerialLinkConfig, Transport};
use wasm_bindgen_futures::JsFuture;
use web_sys::wasm_bindgen::closure::Closure;
use web_sys::wasm_bindgen::{JsCast, JsValue};

pub type WebSerialPort = web_sys::SerialPort;

const SERIAL_TX_BUF_BYTES: usize = 1600;
const ESPNOW_CHUNK_PAYLOAD_BYTES: usize = 1448;
const BLE_CHUNK_PAYLOAD_BYTES: usize = 224;
const POLAR_LEDS: usize = 26;
const POLAR_RADIALS: usize = 360;

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
    let packet = encode_command_packet(command)?;
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
        encode_gif_video_payload_from_bytes(&file, polar, gif_max_fps)?
    } else {
        encode_image_payload_from_bytes(&file, polar)?
    };

    let kind = if is_gif {
        DownloadKind::Video
    } else {
        DownloadKind::DisplayImage
    };

    let packets = chunk_download_payload(&payload, kind, max_chunk_payload(config.transport))?;
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
    )?;

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

fn encode_command_packet(command: SpokeCommand) -> Result<Vec<u8>, String> {
    let mut chunk_buf = [0u8; SERIAL_TX_BUF_BYTES];
    let n = encode_packet(
        Packet::Command(CommandFrame {
            transfer_id: 1,
            command,
        }),
        &mut chunk_buf,
    )
    .map_err(|e| format!("Failed to encode command: {e:?}"))?;

    Ok(chunk_buf[..n].to_vec())
}

fn transport_selector(transport: Transport) -> TransportSelector {
    match transport {
        Transport::Ble => TransportSelector::BleExtAdv,
        Transport::Espnow => TransportSelector::EspNow,
    }
}

fn max_chunk_payload(transport: Transport) -> usize {
    match transport {
        Transport::Ble => BLE_CHUNK_PAYLOAD_BYTES,
        Transport::Espnow => ESPNOW_CHUNK_PAYLOAD_BYTES,
    }
}

fn esp_now_target(delivery: EspNowDelivery) -> EspNowTarget {
    match delivery {
        EspNowDelivery::Broadcast => EspNowTarget::Broadcast,
        EspNowDelivery::Peer(mac) => EspNowTarget::Peer(mac),
    }
}

fn browser_serial() -> Result<web_sys::Serial, String> {
    let window = web_sys::window().ok_or_else(|| "window is not available".to_string())?;
    let navigator = window.navigator();

    let has_serial = Reflect::has(&navigator, &JsValue::from_str("serial")).map_err(js_err)?;
    if !has_serial {
        return Err(
            "Web Serial API is not available. Use a Chromium-based browser over HTTPS or localhost."
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
    let transport_selector = transport_selector(config.transport);
    let esp_now_target = esp_now_target(config.esp_now_delivery);
    let repeat = config.repeat.max(1);

    for _ in 0..repeat {
        for packet in packets {
            let frame = BridgeFrame::data(
                transport_selector,
                esp_now_target,
                config.esp_now_retries,
                packet,
            );

            let bytes = postcard::to_stdvec_cobs(&frame).map_err(|e| e.to_string())?;
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

fn chunk_download_payload(
    payload: &[u8],
    kind: DownloadKind,
    max_chunk_payload: usize,
) -> Result<Vec<Vec<u8>>, String> {
    let mut chunk_buf = [0u8; SERIAL_TX_BUF_BYTES];
    let iter = ChunkIter::new(payload, kind, 1, max_chunk_payload)
        .ok_or_else(|| "Payload too large for transfer format".to_string())?;

    let mut packets: Vec<Vec<u8>> = Vec::new();
    for chunk in iter {
        let n = encode_packet(Packet::Download(chunk), &mut chunk_buf)
            .map_err(|e| format!("Download packet encode failed: {e:?}"))?;
        packets.push(chunk_buf[..n].to_vec());
    }

    Ok(packets)
}

fn encode_image_payload_from_bytes(
    file: &SelectedWebFile,
    polar: Option<PolarEncodeOptions>,
) -> Result<Vec<u8>, String> {
    let image = image::load_from_memory(&file.bytes)
        .map_err(|e| format!("Failed to decode image {}: {e}", file.name))?;

    match polar {
        Some(options) => encode_polar_image(&image.into_rgba8(), options),
        None => encode_cartesian_image(&image),
    }
}

fn encode_cartesian_image(image: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let resized = image.resize_exact(64, 64, image::imageops::FilterType::Lanczos3);
    let pixels: Vec<u8> = resized.to_rgb8().into_raw();
    encode_rgb888_to_wire(&pixels)
        .map_err(|e| format!("Failed to encode image to wire format: {e:?}"))
}

fn encode_polar_image(
    rgba: &image::RgbaImage,
    options: PolarEncodeOptions,
) -> Result<Vec<u8>, String> {
    validate_polar_options(options)?;

    let first_radius = options.first_led_distance / options.last_led_distance;
    let radius_values = evenly_spaced_radii(POLAR_LEDS, first_radius, 1.0);
    let polar_bitmap =
        pov_images::polar_from_image::<POLAR_LEDS, POLAR_RADIALS>(rgba, &radius_values);

    let mut raw: Vec<u8> = Vec::with_capacity(POLAR_LEDS * POLAR_RADIALS * 3);
    for strip in &polar_bitmap.pixels {
        for px in strip {
            raw.push(px.red);
            raw.push(px.green);
            raw.push(px.blue);
        }
    }

    encode_polar_rgb888_to_wire(
        &raw,
        LedCount::new(POLAR_LEDS as u8),
        RadialCount::new(POLAR_RADIALS as u16),
    )
    .map_err(|e| format!("Failed to encode polar image: {e:?}"))
}

fn encode_gif_video_payload_from_bytes(
    file: &SelectedWebFile,
    polar: Option<PolarEncodeOptions>,
    max_fps: Option<u16>,
) -> Result<Vec<u8>, String> {
    if let Some(opts) = polar {
        validate_polar_options(opts)?;
    }
    if let Some(fps) = max_fps {
        if fps == 0 {
            return Err("GIF max FPS must be greater than 0".to_string());
        }
    }

    let cursor = std::io::Cursor::new(file.bytes.as_slice());
    let decoder = image::codecs::gif::GifDecoder::new(cursor)
        .map_err(|e| format!("Failed to decode GIF {}: {e}", file.name))?;

    let frames = decoder
        .into_frames()
        .collect_frames()
        .map_err(|e| format!("Failed to decode GIF frames {}: {e}", file.name))?;

    if frames.is_empty() {
        return Err(format!("GIF contains no frames: {}", file.name));
    }
    if frames.len() > u16::MAX as usize {
        return Err(format!("GIF has too many frames ({})", frames.len()));
    }

    let base_frame_delay_ms = frames
        .iter()
        .map(|f| f.delay().numer_denom_ms().0)
        .find(|ms| *ms > 0)
        .unwrap_or(100);

    let (frame_stride, frame_delay_ms) = if let Some(max_fps) = max_fps {
        let min_delay_ms = 1000u32.div_ceil(max_fps as u32);
        let stride = min_delay_ms.div_ceil(base_frame_delay_ms).max(1) as usize;
        let delay_ms = base_frame_delay_ms
            .saturating_mul(stride as u32)
            .min(u16::MAX as u32) as u16;
        (stride, delay_ms)
    } else {
        (1usize, base_frame_delay_ms.min(u16::MAX as u32) as u16)
    };

    let mut encoded_frames: Vec<Vec<u8>> = Vec::with_capacity(frames.len().div_ceil(frame_stride));
    for frame in frames.iter().step_by(frame_stride) {
        let rgba = frame.buffer().clone();
        let encoded = match polar {
            Some(options) => encode_polar_image(&rgba, options)?,
            None => encode_cartesian_image(&image::DynamicImage::ImageRgba8(rgba))?,
        };
        encoded_frames.push(encoded);
    }

    if encoded_frames.is_empty() {
        return Err(format!(
            "GIF frame selection produced no frames: {}",
            file.name
        ));
    }

    let total_frame_bytes: usize = encoded_frames.iter().map(|f| 4usize + f.len()).sum();
    let mut out = Vec::with_capacity(video::HEADER_LEN + total_frame_bytes);

    out.extend_from_slice(&video::MAGIC);
    out.push(video::WIRE_VERSION);
    out.extend_from_slice(&frame_delay_ms.to_le_bytes());
    out.extend_from_slice(&(encoded_frames.len() as u16).to_le_bytes());

    for frame in &encoded_frames {
        if frame.len() > u32::MAX as usize {
            return Err(format!("Single frame too large: {} bytes", frame.len()));
        }
        out.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        out.extend_from_slice(frame);
    }

    Ok(out)
}

fn validate_polar_options(options: PolarEncodeOptions) -> Result<(), String> {
    if !options.first_led_distance.is_finite() || !options.last_led_distance.is_finite() {
        return Err("LED distances must be finite numbers".to_string());
    }
    if options.first_led_distance < 0.0 || options.last_led_distance <= 0.0 {
        return Err(
            "LED distances must satisfy first_led_distance >= 0 and last_led_distance > 0"
                .to_string(),
        );
    }
    if options.first_led_distance > options.last_led_distance {
        return Err("first_led_distance must be <= last_led_distance".to_string());
    }
    Ok(())
}

fn evenly_spaced_radii(led_count: usize, start: f32, end: f32) -> Vec<f32> {
    if led_count == 0 {
        return Vec::new();
    }
    if led_count == 1 {
        return vec![start];
    }

    let denom = (led_count - 1) as f32;
    (0..led_count)
        .map(|i| {
            let t = (i as f32) / denom;
            start + (end - start) * t
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
struct SendStats {
    packet_count: usize,
    total_transmissions: usize,
}

fn js_err(value: JsValue) -> String {
    value.as_string().unwrap_or_else(|| format!("{value:?}"))
}
