use std::{fs, io::Write, path::Path, thread, time::Duration};

use anyhow::Context;
use pov_proto::{
    bridge::{BridgeControlRequest, BridgeControlResponse},
    transfer::{AdcDevice, AdcSample, SpokeCommand, SpokeResponse, parse_packet},
};
use serialport::SerialPort;

use crate::serial_link::open_serial_port;
use crate::{
    DeviceStorageStats, DownloadKind, DownloadRequest, EspNowDelivery, EspNowPeer,
    PolarEncodeOptions, SendStats, SensorOffsets, SerialLinkConfig, Transport,
    chunk_download_payload, encode_bridge_frame, encode_command_packet,
    encode_command_packet_with_transfer_id, encode_image_path, encode_video_path,
    max_chunk_payload,
};

const RX_BUF: usize = 2048;
const LIST_PEERS_RETRY_DELAY_MS: u64 = 250;

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
    let request =
        pov_proto::bridge::BridgeFrame::ControlRequest(BridgeControlRequest::ListEspNowPeers);
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

fn require_stateful_espnow_target(
    config: &SerialLinkConfig,
    operation: &str,
) -> anyhow::Result<[u8; 6]> {
    if config.transport != Transport::Espnow {
        anyhow::bail!("{operation} requires espnow transport");
    }

    match config.esp_now_delivery {
        EspNowDelivery::Peer(peer) => Ok(peer),
        EspNowDelivery::Broadcast => {
            anyhow::bail!("{operation} requires stateful peer target")
        }
    }
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
    let wire_bytes = encode_image_path(image_path, polar)?;

    let packets = chunk_download_payload(
        &wire_bytes,
        DownloadKind::DisplayImage,
        max_chunk_payload(config.transport),
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
        chunk_download_payload(&payload, request.kind, max_chunk_payload(config.transport))?;
    send_packets(config, &packets)
}

pub fn send_video(
    config: &SerialLinkConfig,
    gif_path: &Path,
    polar: Option<PolarEncodeOptions>,
) -> anyhow::Result<SendStats> {
    send_video_with_max_fps(config, gif_path, polar, None)
}

pub fn send_video_with_max_fps(
    config: &SerialLinkConfig,
    gif_path: &Path,
    polar: Option<PolarEncodeOptions>,
    max_fps: Option<u16>,
) -> anyhow::Result<SendStats> {
    let payload = encode_video_path(gif_path, polar, max_fps)?;
    let packets = chunk_download_payload(
        &payload,
        DownloadKind::Video,
        max_chunk_payload(config.transport),
    )?;
    send_packets(config, &packets)
}

pub fn send_command(config: &SerialLinkConfig, command: SpokeCommand) -> anyhow::Result<SendStats> {
    let packets = vec![encode_command_packet(command)?];
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
    let target_peer = require_stateful_espnow_target(config, "Storage stats request")?;

    let transfer_id = 0x53544154usize; // 'STAT'
    let chunk_buf =
        encode_command_packet_with_transfer_id(transfer_id, SpokeCommand::RequestStorageStats)?;

    let mut port = open_serial_port(&config.port, config.baud)?;
    let frame = encode_bridge_frame(
        config.transport,
        config.esp_now_delivery,
        config.esp_now_retries,
        &chunk_buf,
    )?;
    port.write_all(&frame)
        .context("Failed to write to serial port")?;
    thread::sleep(Duration::from_millis(config.inter_packet_delay_ms));

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

pub fn request_adc_sample(
    config: &SerialLinkConfig,
    device: AdcDevice,
) -> anyhow::Result<AdcSample> {
    let target_peer = require_stateful_espnow_target(config, "ADC sample request")?;

    let transfer_id = 0x4144_4353usize;
    let chunk_buf = encode_command_packet_with_transfer_id(
        transfer_id,
        SpokeCommand::RequestAdcSample { device },
    )?;

    let mut port = open_serial_port(&config.port, config.baud)?;
    let frame = encode_bridge_frame(
        config.transport,
        config.esp_now_delivery,
        config.esp_now_retries,
        &chunk_buf,
    )?;
    port.write_all(&frame)
        .context("Failed to write to serial port")?;
    thread::sleep(Duration::from_millis(config.inter_packet_delay_ms));

    loop {
        let mut frame = read_bridge_control_response(&mut *port, "ADC sample")?;
        let response = postcard::from_bytes_cobs::<BridgeControlResponse<'_>>(&mut frame)
            .context("Failed to decode bridge ADC-sample response")?;
        match response {
            BridgeControlResponse::EspNowPeers(_) => {}
            BridgeControlResponse::EspNowInboundPacket { src, payload } => {
                if src != target_peer {
                    continue;
                }

                let packet = parse_packet(payload)
                    .map_err(|e| anyhow::anyhow!("Failed to parse inbound packet: {:?}", e))?;
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

fn send_packets(config: &SerialLinkConfig, packets: &[Vec<u8>]) -> anyhow::Result<SendStats> {
    let repeat = config.repeat.max(1);
    let mut port = open_serial_port(&config.port, config.baud)?;

    for _ in 0..repeat {
        for packet in packets {
            let frame = encode_bridge_frame(
                config.transport,
                config.esp_now_delivery,
                config.esp_now_retries,
                packet,
            )?;

            port.write_all(&frame)
                .context("Failed to write to serial port")?;
            thread::sleep(Duration::from_millis(config.inter_packet_delay_ms));
        }
    }

    Ok(SendStats {
        packet_count: packets.len(),
        total_transmissions: packets.len() * repeat,
    })
}
