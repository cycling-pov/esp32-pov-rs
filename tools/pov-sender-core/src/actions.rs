use std::{fs, path::Path, thread, time::Duration};

use anyhow::Context;
use pov_proto::{
    bridge::{BridgeFrame, TransportSelector},
    image::{LedCount, RadialCount, encode_polar_rgb888_to_wire, encode_rgb888_to_wire},
    transfer::{ChunkIter, CommandFrame, DownloadKind, Packet, SpokeCommand, encode_packet},
};
use rand::seq::SliceRandom;
use serialport::SerialPort;

use crate::serial_link::open_serial_port;

const ESPNOW_CHUNK_PAYLOAD_BYTES: usize = 1450;
const BLE_CHUNK_PAYLOAD_BYTES: usize = 224;
const SERIAL_TX_BUF_BYTES: usize = 1600;
const POLAR_LEDS: usize = 30;
const POLAR_RADIALS: usize = 360;

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
    pub repeat: usize,
    pub inter_packet_delay_ms: u64,
}

impl Default for SerialLinkConfig {
    fn default() -> Self {
        Self {
            port: String::new(),
            baud: 115_200,
            transport: Transport::Espnow,
            repeat: 1,
            inter_packet_delay_ms: 1_000,
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

fn send_packets(config: &SerialLinkConfig, packets: &[Vec<u8>]) -> anyhow::Result<SendStats> {
    let repeat = config.repeat.max(1);
    let mut port = open_serial_port(&config.port, config.baud)?;
    let transport_selector = config.transport.transport_selector();
    let mut rng = rand::rng();

    for _ in 0..repeat {
        let mut packet_indices: Vec<usize> = (0..packets.len()).collect();
        packet_indices.shuffle(&mut rng);

        for &idx in &packet_indices {
            send_bridge_frame(
                &mut *port,
                transport_selector,
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
    payload: &[u8],
    inter_packet_delay_ms: u64,
) -> anyhow::Result<()> {
    let frame = BridgeFrame {
        transport: transport_selector,
        payload,
    };

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
