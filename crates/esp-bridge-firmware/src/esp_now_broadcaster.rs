//! ESP-NOW broadcast task.
//!
//! Receives raw chunk payloads from `usb_serial_task` and broadcasts each one
//! via ESP-NOW to the all-nodes broadcast address (FF:FF:FF:FF:FF:FF).

use defmt::info;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Receiver};
use esp_radio::esp_now::{BROADCAST_ADDRESS, EspNow};

use crate::usb_serial::ChunkMsg;

#[embassy_executor::task]
pub async fn esp_now_task(
    mut esp_now: EspNow<'static>,
    receiver: Receiver<'static, CriticalSectionRawMutex, ChunkMsg, 4>,
) {
    info!("ESP-NOW broadcaster task started");

    loop {
        let msg = receiver.receive().await;
        info!(
            "ESP-NOW: sending {} bytes to broadcast address FF:FF:FF:FF:FF:FF",
            msg.len
        );

        // Ignore send errors; best-effort broadcast.
        match esp_now
            .send_async(&BROADCAST_ADDRESS, &msg.buf[..msg.len])
            .await
        {
            Ok(()) => {
                info!("ESP-NOW: packet sent successfully");
            }
            Err(err) => {
                info!("ESP-NOW: send failed: {:?}", err);
            }
        }
    }
}
