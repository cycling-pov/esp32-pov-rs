#[cfg(not(target_arch = "wasm32"))]
mod actions;
mod encode;
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EspNowDelivery {
    Broadcast,
    Peer([u8; 6]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transport {
    Ble,
    Espnow,
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

#[derive(Clone, Copy, Debug)]
pub struct PolarEncodeOptions {
    pub first_led_distance: f32,
    pub last_led_distance: f32,
}

#[derive(Clone, Debug)]
pub struct DownloadRequest<'a> {
    pub file_path: &'a Path,
    pub kind: pov_proto::transfer::DownloadKind,
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

pub type AdcDevice = pov_proto::transfer::AdcDevice;
pub type AdcSample = pov_proto::transfer::AdcSample;

#[cfg(target_arch = "wasm32")]
mod actions_wasm;

#[cfg(not(target_arch = "wasm32"))]
mod serial_link;
#[cfg(target_arch = "wasm32")]
mod serial_link_wasm;

#[cfg(not(target_arch = "wasm32"))]
pub use actions::{
    list_esp_now_peers, request_adc_sample, request_storage_stats, send_command, send_download,
    send_image, send_sensor_offsets, send_video, send_video_with_max_fps,
};
#[cfg(target_arch = "wasm32")]
pub use actions_wasm::{
    list_esp_now_peers, request_adc_sample, request_storage_stats, send_command, send_download,
    send_image, send_sensor_offsets, send_video, send_video_with_max_fps,
};
pub use encode::{
    chunk_download_payload, encode_bridge_frame, encode_command_packet,
    encode_command_packet_with_transfer_id, encode_image_bytes, encode_image_path,
    encode_video_bytes, encode_video_path, evenly_spaced_radii, max_chunk_payload,
    validate_polar_options,
};
pub use pov_proto::transfer::{DownloadKind, EstimatorMode, SpokeCommand};
#[cfg(not(target_arch = "wasm32"))]
pub use serial_link::list_serial_ports;
#[cfg(target_arch = "wasm32")]
pub use serial_link_wasm::list_serial_ports;
