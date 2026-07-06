use std::{fs, io::Write, path::Path, thread, time::Duration};

use anyhow::Context;
use image::AnimationDecoder;
use pov_proto::{
    bridge::{
        BridgeControlRequest, BridgeControlResponse, BridgeFrame, EspNowTarget, TransportSelector,
    },
    image::{encode_polar_rgb888_to_wire, encode_rgb888_to_wire, LedCount, RadialCount},
    transfer::{
        encode_packet, parse_packet, ChunkIter, CommandFrame, DownloadKind, Packet, SpokeCommand,
        SpokeResponse,
    },
    video,
};
use rand::seq::SliceRandom;
use serialport::SerialPort;

use crate::serial_link::open_serial_port;

// Keep this in lock-step with esp-spoke-firmware networking/download.rs
// ESPNOW_MAX_CHUNK_PAYLOAD to satisfy transfer payload-shape validation.
const ESPNOW_CHUNK_PAYLOAD_BYTES: usize = 1448;
const BLE_CHUNK_PAYLOAD_BYTES: usize = 224;
const SERIAL_TX_BUF_BYTES: usize = 1600;
const RX_BUF: usize = 2048;
const POLAR_LEDS: usize = 26;
const POLAR_RADIALS: usize = 360;
const LIST_PEERS_RETRY_DELAY_MS: u64 = 250;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EspNowDelivery {
    Broadcast,
    Peer([u8; 6]),
}

impl EspNowDelivery {
    fn target(self) -> EspNowTarget {
        match self {
            Self::Broadcast => EspNowTarget::Broadcast,
            Self::Peer(mac) => EspNowTarget::Peer(mac),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transport {
    Ble,
    Espnow,
}

impl Transport {
    fn transport_selector(self) -> TransportSelector {
        match self {
            Self::Ble => TransportSelector::BleExtAdv,
            Self::Espnow => TransportSelector::EspNow,
        }
    }

    fn max_chunk_payload(self) -> usize {
        match self {
            Self::Ble => BLE_CHUNK_PAYLOAD_BYTES,
            Self::Espnow => ESPNOW_CHUNK_PAYLOAD_BYTES,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SerialLinkConfig {
    pub port: String,
    pub baud: u32,
    pub transport: Transport,
    pub esp_now_delivery: EspNowDelivery,
    pub esp_now_retries: u8,
    pub repeat: usize,
    pub inter_packet_delay_ms: u64,
}

impl Default for SerialLinkConfig {
    fn default() -> Self {
        Self {
            port: String::new(),
            baud: 115_200,
            transport: Transport::Espnow,
            esp_now_delivery: EspNowDelivery::Broadcast,
            esp_now_retries: 0,
            repeat: 1,
            inter_packet_delay_ms: 1_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EspNowPeer {
    pub mac: [u8; 6],
}

pub fn list_esp_now_peers(port_name: &str, baud: u32) -> anyhow::Result<Vec<EspNowPeer>> {
    match list_esp_now_peers_once(port_name, baud) {
        Ok(peers) => Ok(peers),
        Err(first_err) => {
            thread::sleep(Duration::from_millis(LIST_PEERS_RETRY_DELAY_MS));
            list_esp_now_peers_once(port_name, baud).map_err(|retry_err| {
                anyhow::anyhow!(
                    "Failed to list ESP-NOW peers after retry (delay {} ms). First error: {first_err}. Retry error: {retry_err}",
                    LIST_PEERS_RETRY_DELAY_MS
                )
            })
        }
    }
}

fn list_esp_now_peers_once(port_name: &str, baud: u32) -> anyhow::Result<Vec<EspNowPeer>> {
    let mut port = open_serial_port(port_name, baud)?;
    let request = BridgeFrame::ControlRequest(BridgeControlRequest::ListEspNowPeers);
    let cobs_bytes = postcard::to_stdvec_cobs(&request).context("postcard serialization failed")?;
    port.write_all(&cobs_bytes)
        .context("Failed to write peer-list request to serial port")?;

    loop {
        let mut frame = read_bridge_control_response(&mut *port, "list peers")?;
        let response = postcard::from_bytes_cobs::<BridgeControlResponse<'_>>(&mut frame)
            .context("Failed to decode bridge peer-list response")?;
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
            BridgeControlResponse::EspNowInboundPacket { .. } => {
                // Ignore async inbound packets while waiting for peer-list reply.
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PolarEncodeOptions {
    pub first_led_distance: f32,
    pub last_led_distance: f32,
}

#[derive(Clone, Debug)]
pub struct DownloadRequest<'a> {
    pub file_path: &'a Path,
    pub kind: DownloadKind,
}

#[derive(Clone, Copy, Debug)]
pub struct SendStats {
    pub packet_count: usize,
    pub total_transmissions: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct SensorOffsets {
    pub hall_offset_0_degrees: f32,
    pub hall_offset_1_degrees: f32,
    pub imu_offset_degrees: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct DeviceStorageStats {
    pub total_bytes: u32,
    pub used_bytes: u32,
    pub free_bytes: u32,
    pub image_count: u32,
    pub active_image_id: Option<u32>,
}

fn read_bridge_control_response(
    port: &mut dyn SerialPort,
    context: &str,
) -> anyhow::Result<Vec<u8>> {
    let mut response_buf = [0u8; RX_BUF];
    let mut head = 0usize;

    loop {
        let mut byte = [0u8; 1];
        port.read_exact(&mut byte)
            .with_context(|| format!("Timed out waiting for bridge response ({context})"))?;

        if byte[0] == 0 {
            if head == 0 {
                continue;
            }

            return Ok(response_buf[..head].to_vec());
        }

        if head < response_buf.len() {
            response_buf[head] = byte[0];
            head += 1;
        } else {
            anyhow::bail!("Bridge response exceeded buffer size ({context})");
        }
    }
}

pub fn send_image(
    config: &SerialLinkConfig,
    image_path: &Path,
    polar: Option<PolarEncodeOptions>,
) -> anyhow::Result<SendStats> {
    let wire_bytes = match polar {
        Some(polar_opts) => encode_polar_image(image_path, polar_opts)?,
        None => encode_cartesian_image(image_path)?,
    };

    let packets = chunk_download_payload(
        &wire_bytes,
        DownloadKind::DisplayImage,
        config.transport.max_chunk_payload(),
    )?;

    send_packets(config, &packets)
}

pub fn send_download(
    config: &SerialLinkConfig,
    request: DownloadRequest<'_>,
) -> anyhow::Result<SendStats> {
    let payload = fs::read(request.file_path)
        .with_context(|| format!("Failed to read payload file {:?}", request.file_path))?;

    let packets =
        chunk_download_payload(&payload, request.kind, config.transport.max_chunk_payload())?;
    send_packets(config, &packets)
}

pub fn send_video(
    config: &SerialLinkConfig,
    gif_path: &Path,
    polar: Option<PolarEncodeOptions>,
) -> anyhow::Result<SendStats> {
    if let Some(opts) = polar {
        validate_polar_options(opts)?;
    }
    let payload = encode_gif_video_payload(gif_path, polar)?;
    let packets = chunk_download_payload(
        &payload,
        DownloadKind::Video,
        config.transport.max_chunk_payload(),
    )?;
    send_packets(config, &packets)
}

pub fn send_command(config: &SerialLinkConfig, command: SpokeCommand) -> anyhow::Result<SendStats> {
    let mut chunk_buf = [0u8; SERIAL_TX_BUF_BYTES];

    let n = encode_packet(
        Packet::Command(CommandFrame {
            transfer_id: 1,
            command,
        }),
        &mut chunk_buf,
    )
    .map_err(|e| anyhow::anyhow!("Failed to encode command: {:?}", e))?;

    let packets = vec![chunk_buf[..n].to_vec()];
    send_packets(config, &packets)
}

pub fn send_sensor_offsets(
    config: &SerialLinkConfig,
    offsets: SensorOffsets,
) -> anyhow::Result<SendStats> {
    send_command(
        config,
        SpokeCommand::SetSensorOffsets {
            hall_offset_0_degrees: offsets.hall_offset_0_degrees,
            hall_offset_1_degrees: offsets.hall_offset_1_degrees,
            imu_offset_degrees: offsets.imu_offset_degrees,
        },
    )
}

pub fn request_storage_stats(config: &SerialLinkConfig) -> anyhow::Result<DeviceStorageStats> {
    if config.transport != Transport::Espnow {
        anyhow::bail!("Storage stats request requires espnow transport");
    }

    let target_peer = match config.esp_now_delivery {
        EspNowDelivery::Peer(peer) => peer,
        EspNowDelivery::Broadcast => {
            anyhow::bail!("Storage stats request requires stateful peer target")
        }
    };

    let mut chunk_buf = [0u8; SERIAL_TX_BUF_BYTES];
    let transfer_id = 0x53544154usize; // 'STAT'
    let n = encode_packet(
        Packet::Command(CommandFrame {
            transfer_id,
            command: SpokeCommand::RequestStorageStats,
        }),
        &mut chunk_buf,
    )
    .map_err(|e| anyhow::anyhow!("Failed to encode stats command: {:?}", e))?;

    let mut port = open_serial_port(&config.port, config.baud)?;
    send_bridge_frame(
        &mut *port,
        TransportSelector::EspNow,
        EspNowTarget::Peer(target_peer),
        config.esp_now_retries,
        &chunk_buf[..n],
        config.inter_packet_delay_ms,
    )?;

    loop {
        let mut frame = read_bridge_control_response(&mut *port, "storage stats")?;
        let response = postcard::from_bytes_cobs::<BridgeControlResponse<'_>>(&mut frame)
            .context("Failed to decode bridge storage-stats response")?;
        match response {
            BridgeControlResponse::EspNowPeers(_) => {}
            BridgeControlResponse::EspNowInboundPacket { src, payload } => {
                if src != target_peer {
                    continue;
                }

                let packet = parse_packet(payload)
                    .map_err(|e| anyhow::anyhow!("Failed to parse inbound packet: {:?}", e))?;
                match packet {
                    Packet::Response(frame) if frame.transfer_id == transfer_id => {
                        let SpokeResponse::StorageStats(stats) = frame.response;
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

fn send_packets(config: &SerialLinkConfig, packets: &[Vec<u8>]) -> anyhow::Result<SendStats> {
    let repeat = config.repeat.max(1);
    let mut port = open_serial_port(&config.port, config.baud)?;
    let transport_selector = config.transport.transport_selector();
    let esp_now_target = config.esp_now_delivery.target();
    let mut rng = rand::rng();

    for _ in 0..repeat {
        let mut packet_indices: Vec<usize> = (0..packets.len()).collect();
        packet_indices.shuffle(&mut rng);

        for &idx in &packet_indices {
            send_bridge_frame(
                &mut *port,
                transport_selector,
                esp_now_target,
                config.esp_now_retries,
                &packets[idx],
                config.inter_packet_delay_ms,
            )?;
        }
    }

    Ok(SendStats {
        packet_count: packets.len(),
        total_transmissions: packets.len() * repeat,
    })
}

fn send_bridge_frame(
    port: &mut dyn SerialPort,
    transport_selector: TransportSelector,
    esp_now_target: EspNowTarget,
    esp_now_retries: u8,
    payload: &[u8],
    inter_packet_delay_ms: u64,
) -> anyhow::Result<()> {
    let frame = BridgeFrame::data(transport_selector, esp_now_target, esp_now_retries, payload);

    let cobs_bytes = postcard::to_stdvec_cobs(&frame).context("postcard serialization failed")?;

    port.write_all(&cobs_bytes)
        .context("Failed to write to serial port")?;

    thread::sleep(Duration::from_millis(inter_packet_delay_ms));

    Ok(())
}

fn chunk_download_payload(
    payload: &[u8],
    kind: DownloadKind,
    max_chunk_payload: usize,
) -> anyhow::Result<Vec<Vec<u8>>> {
    let mut chunk_buf = [0u8; SERIAL_TX_BUF_BYTES];
    let iter = ChunkIter::new(payload, kind, 1, max_chunk_payload)
        .expect("Payload too large for pov-proto transfer");

    let mut packets: Vec<Vec<u8>> = Vec::new();
    for chunk in iter {
        let n = encode_packet(Packet::Download(chunk), &mut chunk_buf).map_err(|e| {
            anyhow::anyhow!(
                "encode failed: {:?}; payload_len={}, chunk_index={}, chunk_count={}, total_len={}, max_chunk_payload={}",
                e,
                chunk.payload.len(),
                chunk.chunk_index,
                chunk.chunk_count,
                chunk.total_len,
                max_chunk_payload,
            )
        })?;
        packets.push(chunk_buf[..n].to_vec());
    }

    Ok(packets)
}

fn encode_cartesian_image(image_path: &Path) -> anyhow::Result<Vec<u8>> {
    let img = image::open(image_path)
        .with_context(|| format!("Failed to open image {:?}", image_path))?;
    let resized = img.resize_exact(64, 64, image::imageops::FilterType::Lanczos3);
    let pixels: Vec<u8> = resized.to_rgb8().into_raw();

    encode_rgb888_to_wire(&pixels)
        .map_err(|e| anyhow::anyhow!("Failed to encode image to wire format: {:?}", e))
}

fn encode_polar_image(image_path: &Path, options: PolarEncodeOptions) -> anyhow::Result<Vec<u8>> {
    validate_polar_options(options)?;

    let first_radius = options.first_led_distance / options.last_led_distance;
    let radius_values = evenly_spaced_radii(POLAR_LEDS, first_radius, 1.0);

    let img = image::open(image_path)
        .with_context(|| format!("Failed to open image {:?}", image_path))?
        .into_rgba8();

    let polar_bitmap =
        pov_images::polar_from_image::<POLAR_LEDS, POLAR_RADIALS>(&img, &radius_values);

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
    .map_err(|e| anyhow::anyhow!("Failed to encode polar image: {:?}", e))
}

fn validate_polar_options(options: PolarEncodeOptions) -> anyhow::Result<()> {
    if !options.first_led_distance.is_finite() || !options.last_led_distance.is_finite() {
        anyhow::bail!("LED distances must be finite numbers");
    }

    if options.first_led_distance < 0.0 || options.last_led_distance <= 0.0 {
        anyhow::bail!(
            "LED distances must satisfy first_led_distance >= 0 and last_led_distance > 0"
        );
    }

    if options.first_led_distance > options.last_led_distance {
        anyhow::bail!("first_led_distance must be <= last_led_distance");
    }

    Ok(())
}

fn encode_gif_video_payload(
    gif_path: &Path,
    polar: Option<PolarEncodeOptions>,
) -> anyhow::Result<Vec<u8>> {
    let gif_file = std::fs::File::open(gif_path)
        .with_context(|| format!("Failed to open GIF {:?}", gif_path))?;
    let reader = std::io::BufReader::new(gif_file);
    let decoder = image::codecs::gif::GifDecoder::new(reader)
        .with_context(|| format!("Failed to decode GIF {:?}", gif_path))?;

    let frames = decoder
        .into_frames()
        .collect_frames()
        .with_context(|| format!("Failed to decode GIF frames {:?}", gif_path))?;

    anyhow::ensure!(!frames.is_empty(), "GIF contains no frames: {:?}", gif_path);
    anyhow::ensure!(
        frames.len() <= u16::MAX as usize,
        "GIF has too many frames ({})",
        frames.len()
    );

    // GIF delays are in units of 10 ms. Use the first non-zero delay and keep
    // a fixed cadence in the wire format.
    let frame_delay_ms = frames
        .iter()
        .map(|f| f.delay().numer_denom_ms().0)
        .find(|ms| *ms > 0)
        .unwrap_or(100)
        .min(u16::MAX as u32) as u16;

    let mut encoded_frames: Vec<Vec<u8>> = Vec::with_capacity(frames.len());
    for frame in &frames {
        let rgba = frame.buffer().clone();
        let encoded = match polar {
            Some(opts) => encode_polar_rgba(&rgba, opts)?,
            None => encode_cartesian_rgba(&rgba)?,
        };
        encoded_frames.push(encoded);
    }

    let total_frame_bytes: usize = encoded_frames
        .iter()
        .map(|f| 4usize.saturating_add(f.len()))
        .sum();
    let mut out = Vec::with_capacity(video::HEADER_LEN + total_frame_bytes);

    out.extend_from_slice(&video::MAGIC);
    out.push(video::WIRE_VERSION);
    out.extend_from_slice(&frame_delay_ms.to_le_bytes());
    out.extend_from_slice(&(encoded_frames.len() as u16).to_le_bytes());

    for frame in &encoded_frames {
        anyhow::ensure!(
            frame.len() <= u32::MAX as usize,
            "Single frame too large: {} bytes",
            frame.len()
        );
        out.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        out.extend_from_slice(frame);
    }

    Ok(out)
}

fn encode_cartesian_rgba(rgba: &image::RgbaImage) -> anyhow::Result<Vec<u8>> {
    let img = image::DynamicImage::ImageRgba8(rgba.clone());
    let resized = img.resize_exact(64, 64, image::imageops::FilterType::Lanczos3);
    let pixels: Vec<u8> = resized.to_rgb8().into_raw();
    encode_rgb888_to_wire(&pixels)
        .map_err(|e| anyhow::anyhow!("Failed to encode frame to wire format: {:?}", e))
}

fn encode_polar_rgba(
    rgba: &image::RgbaImage,
    options: PolarEncodeOptions,
) -> anyhow::Result<Vec<u8>> {
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
    .map_err(|e| anyhow::anyhow!("Failed to encode polar frame: {:?}", e))
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

#[cfg(test)]
mod tests {
    use super::evenly_spaced_radii;

    #[test]
    fn evenly_spaced_radii_endpoints_are_preserved() {
        let radii = evenly_spaced_radii(4, 0.25, 1.0);
        assert_eq!(radii.len(), 4);
        assert!((radii[0] - 0.25).abs() < f32::EPSILON);
        assert!((radii[3] - 1.0).abs() < f32::EPSILON);
    }
}
