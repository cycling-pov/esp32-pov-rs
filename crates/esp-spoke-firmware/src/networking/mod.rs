use alloc::boxed::Box;

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use pov_proto::transfer::CommandFrame;

#[cfg(feature = "ble")]
pub mod ble;
mod download;
#[cfg(feature = "espnow")]
pub mod esp_now;

pub use download::{
    BLE_MAX_CHUNK_PAYLOAD, ESPNOW_MAX_CHUNK_PAYLOAD, IngestError, MAX_TRANSFER_BYTES,
};
#[cfg(any(feature = "ble", feature = "espnow"))]
use static_cell::StaticCell;

pub type CompletedDownload = download::CompletedDownload;

static DOWNLOAD_CHANNEL: Channel<CriticalSectionRawMutex, Box<CompletedDownload>, 2> =
    Channel::new();
static COMMAND_CHANNEL: Channel<CriticalSectionRawMutex, CommandFrame, 4> = Channel::new();

pub async fn receive_download() -> Option<Box<CompletedDownload>> {
    DOWNLOAD_CHANNEL.receive().await.into()
}

pub async fn receive_command() -> Option<CommandFrame> {
    COMMAND_CHANNEL.receiver().receive().await.into()
}

#[cfg(any(feature = "ble", feature = "espnow"))]
static RADIO_CONTROLLER: StaticCell<esp_radio::Controller<'static>> = StaticCell::new();
#[cfg(feature = "espnow")]
static WIFI_CONTROLLER: StaticCell<esp_radio::wifi::WifiController<'static>> = StaticCell::new();

pub async fn init(
    _wifi: esp_hal::peripherals::WIFI<'static>,
    _bluetooth: esp_hal::peripherals::BT<'static>,
    spawner: Spawner,
) {
    #[cfg(any(feature = "ble", feature = "espnow"))]
    let radio =
        RADIO_CONTROLLER.init(esp_radio::init().expect("failed to initialize radio controller"));

    #[cfg(feature = "espnow")]
    {
        let (mut wifi_ctrl, interfaces) = esp_radio::wifi::new(radio, _wifi, Default::default())
            .expect("failed to initialize WiFi");
        wifi_ctrl
            .set_mode(esp_radio::wifi::WifiMode::Sta)
            .expect("failed to set WiFi mode");
        info!("WiFi mode set to STA, starting WiFi...");
        wifi_ctrl.start_async().await.expect("failed to start WiFi");
        info!("WiFi started, configuring ESP-NOW...");
        let esp_now = interfaces.esp_now;

        // Set explicit WiFi channel to ensure spoke and bridge sync on the same channel.
        const ESPNOW_CHANNEL: u8 = 6;
        esp_now
            .set_channel(ESPNOW_CHANNEL)
            .expect("failed to set ESP-NOW channel");
        info!("ESP-NOW channel set to {}", ESPNOW_CHANNEL);

        // Keep `wifi_ctrl` alive — dropping it would call `esp_wifi_stop()`.
        let _wifi_ctrl = WIFI_CONTROLLER.init(wifi_ctrl);
        esp_now::start_esp_now_backend(spawner, esp_now);
    }
    #[cfg(feature = "ble")]
    {
        let ble_connector =
            esp_radio::ble::controller::BleConnector::new(radio, _bluetooth, Default::default())
                .expect("failed to initialize BLE connector");
        let ble_controller: bt_hci::controller::ExternalController<_, 1> =
            bt_hci::controller::ExternalController::new(ble_connector);
        ble::start_ble_backend(spawner, ble_controller);
    }
}

#[cfg(feature = "ble")]
pub fn ingest_ble_payload(payload: &[u8]) -> Result<(), IngestError> {
    route_ingested_packet(download::ingest_ble_payload(payload)?)
}

#[cfg(feature = "espnow")]
pub fn ingest_espnow_payload(payload: &[u8]) -> Result<(), IngestError> {
    route_ingested_packet(download::ingest_espnow_payload(payload)?)
}

fn route_ingested_packet(packet: Option<download::IngestedPacket>) -> Result<(), IngestError> {
    if let Some(packet) = packet {
        match packet {
            download::IngestedPacket::Download(completed) => {
                let kind = completed.kind;
                let transfer_id = completed.transfer_id;
                let crc32 = completed.crc32;
                let byte_len = completed.len;

                if DOWNLOAD_CHANNEL.sender().try_send(completed).is_err() {
                    warn!("dropping completed download: channel full");
                } else {
                    info!(
                        "queued completed download: kind={:?} transfer_id={=usize} bytes={=usize} crc32={=u32}",
                        kind, transfer_id, byte_len, crc32
                    );
                }
            }
            download::IngestedPacket::Command(frame) => {
                let transfer_id = frame.transfer_id;
                let command = frame.command;

                if COMMAND_CHANNEL.sender().try_send(frame).is_err() {
                    warn!("dropping command packet: channel full");
                } else {
                    info!(
                        "queued command packet: transfer_id={=usize} command={:?}",
                        transfer_id, command
                    );
                }
            }
        }
    }

    Ok(())
}
