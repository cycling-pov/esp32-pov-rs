use std::path::Path;

use anyhow::Context;
use image::AnimationDecoder;
use pov_proto::{
    bridge::{BridgeFrame, EspNowTarget, TransportSelector},
    image::{LedCount, RadialCount, encode_polar_rgb888_to_wire, encode_rgb888_to_wire},
    transfer::{ChunkIter, CommandFrame, DownloadKind, Packet, SpokeCommand, encode_packet},
    video,
};

use crate::{EspNowDelivery, PolarEncodeOptions, Transport};

const ESPNOW_CHUNK_PAYLOAD_BYTES: usize = 1448;
const BLE_CHUNK_PAYLOAD_BYTES: usize = 224;
const SERIAL_TX_BUF_BYTES: usize = 1600;
const POLAR_LEDS: usize = 26;
const POLAR_RADIALS: usize = 360;

pub fn max_chunk_payload(transport: Transport) -> usize {
    match transport {
        Transport::Ble => BLE_CHUNK_PAYLOAD_BYTES,
        Transport::Espnow => ESPNOW_CHUNK_PAYLOAD_BYTES,
    }
}

pub fn encode_bridge_frame(
    transport: Transport,
    esp_now_delivery: EspNowDelivery,
    esp_now_retries: u8,
    payload: &[u8],
) -> anyhow::Result<Vec<u8>> {
    let frame = BridgeFrame::data(
        match transport {
            Transport::Ble => TransportSelector::BleExtAdv,
            Transport::Espnow => TransportSelector::EspNow,
        },
        match esp_now_delivery {
            EspNowDelivery::Broadcast => EspNowTarget::Broadcast,
            EspNowDelivery::Peer(mac) => EspNowTarget::Peer(mac),
        },
        esp_now_retries,
        payload,
    );

    postcard::to_stdvec_cobs(&frame).context("postcard serialization failed")
}

pub fn encode_command_packet(command: SpokeCommand) -> anyhow::Result<Vec<u8>> {
    encode_command_packet_with_transfer_id(1, command)
}

pub fn encode_command_packet_with_transfer_id(
    transfer_id: usize,
    command: SpokeCommand,
) -> anyhow::Result<Vec<u8>> {
    let mut chunk_buf = [0u8; SERIAL_TX_BUF_BYTES];
    let n = encode_packet(
        Packet::Command(CommandFrame {
            transfer_id,
            command,
        }),
        &mut chunk_buf,
    )
    .map_err(|e| anyhow::anyhow!("Failed to encode command: {:?}", e))?;

    Ok(chunk_buf[..n].to_vec())
}

pub fn chunk_download_payload(
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

pub fn encode_image_path(
    image_path: &Path,
    polar: Option<PolarEncodeOptions>,
) -> anyhow::Result<Vec<u8>> {
    let image = image::open(image_path)
        .with_context(|| format!("Failed to open image {:?}", image_path))?;
    encode_dynamic_image(image, polar)
}

pub fn encode_image_bytes(
    image_name: &str,
    image_bytes: &[u8],
    polar: Option<PolarEncodeOptions>,
) -> anyhow::Result<Vec<u8>> {
    let image = image::load_from_memory(image_bytes)
        .with_context(|| format!("Failed to decode image {image_name}"))?;
    encode_dynamic_image(image, polar)
}

pub fn encode_video_path(
    gif_path: &Path,
    polar: Option<PolarEncodeOptions>,
    max_fps: Option<u16>,
) -> anyhow::Result<Vec<u8>> {
    let gif_file = std::fs::File::open(gif_path)
        .with_context(|| format!("Failed to open GIF {:?}", gif_path))?;
    let reader = std::io::BufReader::new(gif_file);
    encode_video_reader(&format!("{:?}", gif_path), reader, polar, max_fps)
}

pub fn encode_video_bytes(
    file_name: &str,
    file_bytes: &[u8],
    polar: Option<PolarEncodeOptions>,
    max_fps: Option<u16>,
) -> anyhow::Result<Vec<u8>> {
    let cursor = std::io::Cursor::new(file_bytes);
    encode_video_reader(file_name, cursor, polar, max_fps)
}

fn encode_dynamic_image(
    image: image::DynamicImage,
    polar: Option<PolarEncodeOptions>,
) -> anyhow::Result<Vec<u8>> {
    match polar {
        Some(options) => encode_polar_image(&image.into_rgba8(), options),
        None => encode_cartesian_image(&image),
    }
}

fn encode_cartesian_image(image: &image::DynamicImage) -> anyhow::Result<Vec<u8>> {
    let resized = image.resize_exact(64, 64, image::imageops::FilterType::Lanczos3);
    let pixels: Vec<u8> = resized.to_rgb8().into_raw();
    encode_rgb888_to_wire(&pixels)
        .map_err(|e| anyhow::anyhow!("Failed to encode image to wire format: {:?}", e))
}

fn encode_polar_image(
    rgba: &image::RgbaImage,
    options: PolarEncodeOptions,
) -> anyhow::Result<Vec<u8>> {
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
    .map_err(|e| anyhow::anyhow!("Failed to encode polar image: {:?}", e))
}

fn encode_video_reader<R: std::io::BufRead + std::io::Seek>(
    file_name: &str,
    reader: R,
    polar: Option<PolarEncodeOptions>,
    max_fps: Option<u16>,
) -> anyhow::Result<Vec<u8>> {
    if let Some(opts) = polar {
        validate_polar_options(opts)?;
    }
    if let Some(fps) = max_fps {
        anyhow::ensure!(fps > 0, "GIF max FPS must be greater than 0");
    }

    let decoder = image::codecs::gif::GifDecoder::new(reader)
        .with_context(|| format!("Failed to decode GIF {file_name}"))?;

    let frames = decoder
        .into_frames()
        .collect_frames()
        .with_context(|| format!("Failed to decode GIF frames {file_name}"))?;

    anyhow::ensure!(!frames.is_empty(), "GIF contains no frames: {file_name}");
    anyhow::ensure!(
        frames.len() <= u16::MAX as usize,
        "GIF has too many frames ({})",
        frames.len()
    );

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

    anyhow::ensure!(
        !encoded_frames.is_empty(),
        "GIF frame selection produced no frames: {file_name}"
    );
    anyhow::ensure!(
        encoded_frames.len() <= u16::MAX as usize,
        "GIF has too many selected frames ({})",
        encoded_frames.len()
    );

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

pub fn validate_polar_options(options: PolarEncodeOptions) -> anyhow::Result<()> {
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

pub fn evenly_spaced_radii(led_count: usize, start: f32, end: f32) -> Vec<f32> {
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
