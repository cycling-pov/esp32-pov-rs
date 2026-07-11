#[cfg(not(target_arch = "wasm32"))]
mod actions;
#[cfg(target_arch = "wasm32")]
mod actions_wasm;

#[cfg(not(target_arch = "wasm32"))]
mod serial_link;
#[cfg(target_arch = "wasm32")]
mod serial_link_wasm;

#[cfg(not(target_arch = "wasm32"))]
pub use actions::{
    DeviceStorageStats, DownloadRequest, EspNowDelivery, EspNowPeer, PolarEncodeOptions, SendStats,
    SensorOffsets, SerialLinkConfig, Transport, list_esp_now_peers, request_adc_sample,
    request_storage_stats, send_command, send_download, send_image, send_sensor_offsets,
    send_video, send_video_with_max_fps,
};
#[cfg(target_arch = "wasm32")]
pub use actions_wasm::{
    DeviceStorageStats, DownloadRequest, EspNowDelivery, EspNowPeer, PolarEncodeOptions, SendStats,
    SensorOffsets, SerialLinkConfig, Transport, list_esp_now_peers, request_adc_sample,
    request_storage_stats, send_command, send_download, send_image, send_sensor_offsets,
    send_video, send_video_with_max_fps,
};
pub use pov_proto::transfer::{AdcDevice, AdcSample, DownloadKind, SpokeCommand};
#[cfg(not(target_arch = "wasm32"))]
pub use serial_link::list_serial_ports;
#[cfg(target_arch = "wasm32")]
pub use serial_link_wasm::list_serial_ports;
