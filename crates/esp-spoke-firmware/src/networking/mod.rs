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
#[cfg(feature = "usb-serial")]
pub mod usb_serial;

pub use download::{
    BLE_MAX_CHUNK_PAYLOAD, ESPNOW_MAX_CHUNK_PAYLOAD, IngestError, MAX_TRANSFER_BYTES, NetworkChunk,
};
#[cfg(feature = "espnow")]
use static_cell::StaticCell;

#[derive(Clone, Copy)]
pub struct ReceivedCommand {
    pub frame: CommandFrame,
    pub source_peer: Option<[u8; 6]>,
}

#[cfg(feature = "espnow")]
pub struct OutboundEspNowPacket {
    pub peer: [u8; 6],
    pub len: usize,
    pub payload: [u8; ESPNOW_MAX_CHUNK_PAYLOAD],
}

/// Channel that carries individual image chunks to the main orchestration loop.
/// Capacity 64 is large enough to buffer a full BLE transfer (≤46 chunks at
/// 224 B each) during the initial flash-erase of the target slot.
static CHUNK_CHANNEL: Channel<CriticalSectionRawMutex, NetworkChunk, 64> = Channel::new();
static COMMAND_CHANNEL: Channel<CriticalSectionRawMutex, ReceivedCommand, 4> = Channel::new();
#[cfg(feature = "espnow")]
pub(super) static ESP_NOW_TX_CHANNEL: Channel<CriticalSectionRawMutex, OutboundEspNowPacket, 4> =
    Channel::new();

pub async fn receive_chunk() -> Option<NetworkChunk> {
    CHUNK_CHANNEL.receive().await.into()
}

pub async fn receive_command() -> Option<ReceivedCommand> {
    COMMAND_CHANNEL.receiver().receive().await.into()
}

#[cfg(feature = "espnow")]
pub async fn send_espnow_packet(peer: [u8; 6], payload: &[u8]) -> Result<(), ()> {
    if payload.len() > ESPNOW_MAX_CHUNK_PAYLOAD {
        return Err(());
    }

    let mut packet = OutboundEspNowPacket {
        peer,
        len: payload.len(),
        payload: [0u8; ESPNOW_MAX_CHUNK_PAYLOAD],
    };
    packet.payload[..payload.len()].copy_from_slice(payload);
    ESP_NOW_TX_CHANNEL.sender().send(packet).await;
    Ok(())
}

#[cfg(feature = "espnow")]
static WIFI_CONTROLLER: StaticCell<esp_radio::wifi::WifiController<'static>> = StaticCell::new();

pub async fn init(
    _wifi: esp_hal::peripherals::WIFI<'static>,
    _bluetooth: esp_hal::peripherals::BT<'static>,
    spawner: Spawner,
) {
    #[cfg(feature = "espnow")]
    {
        let (mut wifi_ctrl, interfaces) =
            esp_radio::wifi::new(_wifi, Default::default()).expect("failed to initialize WiFi");
        wifi_ctrl
            .set_config(&esp_radio::wifi::Config::Station(Default::default()))
            .expect("failed to set WiFi config to STA");
        info!("WiFi mode set to STA, starting WiFi...");
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
            esp_radio::ble::controller::BleConnector::new(_bluetooth, Default::default())
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
pub fn ingest_espnow_payload(payload: &[u8], source_peer: [u8; 6]) -> Result<(), IngestError> {
    route_ingested_packet(download::ingest_espnow_payload(payload, source_peer)?)
}

fn route_ingested_packet(packet: Option<download::IngestedPacket>) -> Result<(), IngestError> {
    if let Some(packet) = packet {
        match packet {
            download::IngestedPacket::Chunk(chunk) => {
                let transfer_id = chunk.transfer_id;
                let byte_offset = chunk.byte_offset;
                let is_final = chunk.is_final;

                if CHUNK_CHANNEL.try_send(chunk).is_err() {
                    warn!(
                        "dropping chunk: channel full transfer_id={=usize} offset={=u32}",
                        transfer_id, byte_offset
                    );
                } else {
                    info!(
                        "queued chunk: transfer_id={=usize} offset={=u32} is_final={=bool}",
                        transfer_id, byte_offset, is_final
                    );
                }
            }
            download::IngestedPacket::Command { frame, source_peer } => {
                let transfer_id = frame.transfer_id;
                let command_kind = frame.command;
                let received_command = ReceivedCommand { frame, source_peer };

                if COMMAND_CHANNEL.sender().try_send(received_command).is_err() {
                    warn!("dropping command packet: channel full");
                } else {
                    info!(
                        "queued command packet: transfer_id={=usize} command={:?}",
                        transfer_id, command_kind
                    );
                }
            }
        }
    }

    Ok(())
}

#[cfg(feature = "usb-serial")]
pub fn start_usb_serial_backend(
    spawner: Spawner,
    usb: esp_hal::usb_serial_jtag::UsbSerialJtag<'static, esp_hal::Async>,
) {
    spawner.spawn(usb_serial::usb_serial_task(usb).unwrap());
}
