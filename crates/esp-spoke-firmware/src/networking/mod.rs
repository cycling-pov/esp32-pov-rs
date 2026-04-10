use alloc::boxed::Box;

use defmt::{info, warn};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use pov_proto::transfer::CommandFrame;

pub mod ble;
mod download;
pub mod esp_now;

pub use download::{IngestError, MAX_CHUNK_PAYLOAD, MAX_TRANSFER_BYTES};

pub type CompletedDownload = download::CompletedDownload;

static DOWNLOAD_CHANNEL: Channel<CriticalSectionRawMutex, Box<CompletedDownload>, 2> =
    Channel::new();
static COMMAND_CHANNEL: Channel<CriticalSectionRawMutex, CommandFrame, 4> = Channel::new();

pub fn try_receive_download() -> Option<Box<CompletedDownload>> {
    DOWNLOAD_CHANNEL.receiver().try_receive().ok()
}

pub fn try_receive_command() -> Option<CommandFrame> {
    COMMAND_CHANNEL.receiver().try_receive().ok()
}

pub fn ingest_manufacturer_data(payload: &[u8]) -> Result<(), IngestError> {
    if let Some(packet) = download::ingest_manufacturer_data(payload)? {
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
