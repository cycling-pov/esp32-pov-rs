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

pub fn list_esp_now_peers(_port_name: &str, _baud: u32) -> anyhow::Result<Vec<EspNowPeer>> {
    anyhow::bail!("ESP-NOW peer listing is not available in wasm sender core")
}

pub fn send_image(
    _config: &SerialLinkConfig,
    _image_path: &Path,
    _polar: Option<PolarEncodeOptions>,
) -> anyhow::Result<SendStats> {
    anyhow::bail!("Image send is not available in wasm sender core")
}

pub fn send_download(
    _config: &SerialLinkConfig,
    _request: DownloadRequest<'_>,
) -> anyhow::Result<SendStats> {
    anyhow::bail!("Download send is not available in wasm sender core")
}

pub fn send_video(
    _config: &SerialLinkConfig,
    _gif_path: &Path,
    _polar: Option<PolarEncodeOptions>,
) -> anyhow::Result<SendStats> {
    anyhow::bail!("Video send is not available in wasm sender core")
}

pub fn send_video_with_max_fps(
    _config: &SerialLinkConfig,
    _gif_path: &Path,
    _polar: Option<PolarEncodeOptions>,
    _max_fps: Option<u16>,
) -> anyhow::Result<SendStats> {
    anyhow::bail!("Video send is not available in wasm sender core")
}

pub fn send_command(
    _config: &SerialLinkConfig,
    _command: pov_proto::transfer::SpokeCommand,
) -> anyhow::Result<SendStats> {
    anyhow::bail!("Command send is not available in wasm sender core")
}

pub fn send_sensor_offsets(
    _config: &SerialLinkConfig,
    _offsets: SensorOffsets,
) -> anyhow::Result<SendStats> {
    anyhow::bail!("Sensor offset send is not available in wasm sender core")
}

pub fn request_storage_stats(_config: &SerialLinkConfig) -> anyhow::Result<DeviceStorageStats> {
    anyhow::bail!("Storage stats request is not available in wasm sender core")
}

pub fn request_adc_sample(
    _config: &SerialLinkConfig,
    _device: AdcDevice,
) -> anyhow::Result<AdcSample> {
    anyhow::bail!("ADC sample request is not available in wasm sender core")
}
