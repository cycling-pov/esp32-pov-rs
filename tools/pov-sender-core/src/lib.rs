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
    list_esp_now_peers, request_storage_stats, send_command, send_download, send_image,
    send_sensor_offsets, send_video, send_video_with_max_fps, DeviceStorageStats, DownloadRequest,
    EspNowDelivery, EspNowPeer, PolarEncodeOptions, SendStats, SensorOffsets, SerialLinkConfig,
    Transport,
};
#[cfg(target_arch = "wasm32")]
pub use actions_wasm::{
    list_esp_now_peers, request_storage_stats, send_command, send_download, send_image,
    send_sensor_offsets, send_video, send_video_with_max_fps, DeviceStorageStats, DownloadRequest,
    EspNowDelivery, EspNowPeer, PolarEncodeOptions, SendStats, SensorOffsets, SerialLinkConfig,
    Transport,
};
pub use pov_proto::transfer::{DownloadKind, SpokeCommand};
#[cfg(not(target_arch = "wasm32"))]
pub use serial_link::list_serial_ports;
#[cfg(target_arch = "wasm32")]
pub use serial_link_wasm::list_serial_ports;
