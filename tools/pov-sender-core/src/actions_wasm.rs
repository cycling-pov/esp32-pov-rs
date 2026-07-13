use std::path::Path;

use crate::{
    AdcDevice, AdcSample, DeviceStorageStats, DownloadRequest, EspNowPeer, PolarEncodeOptions,
    SendStats, SensorOffsets, SerialLinkConfig,
};

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
