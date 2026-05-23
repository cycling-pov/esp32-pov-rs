use defmt::{debug, info, warn};
use embassy_executor::Spawner;
use esp_radio::esp_now::EspNow;

pub fn start_esp_now_backend(spawner: Spawner, esp_now: EspNow<'static>) {
    spawner.spawn(esp_now_backend_task(esp_now).unwrap());
}

#[embassy_executor::task]
pub async fn esp_now_backend_task(mut esp_now: EspNow<'static>) {
    info!("ESP-NOW broadcast receiver backend starting");

    // Print diagnostics about ESP-NOW configuration
    match esp_now.version() {
        Ok(version) => info!("ESP-NOW version: {=u32}", version),
        Err(err) => warn!("Failed to get ESP-NOW version: {:?}", err),
    }

    match esp_now.peer_count() {
        Ok(count) => {
            info!(
                "ESP-NOW peer count: total={=i32} encrypted={=i32}",
                count.total_count, count.encrypted_count
            );
        }
        Err(err) => warn!("Failed to get peer count: {:?}", err),
    }

    info!("Waiting for ESP-NOW packets...");

    let mut consecutive_packets = 0u32;

    loop {
        let received = esp_now.receive_async().await;
        let payload = received.data();
        let src = received.info.src_address;

        debug!(
            "ESP-NOW packet received: src={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} bytes={=usize}",
            src[0],
            src[1],
            src[2],
            src[3],
            src[4],
            src[5],
            payload.len()
        );

        // Track burst traffic for diagnostics
        consecutive_packets += 1;
        if consecutive_packets.is_multiple_of(10) {
            info!(
                "ESP-NOW: received {} packets in active session",
                consecutive_packets
            );
        }

        match super::ingest_espnow_payload(payload) {
            Ok(()) => {
                debug!("ESP-NOW packet successfully ingested");
            }
            Err(err) => {
                warn!(
                    "ESP-NOW packet ingest failed: src={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} bytes={=usize} err={:?}",
                    src[0],
                    src[1],
                    src[2],
                    src[3],
                    src[4],
                    src[5],
                    payload.len(),
                    err
                );
            }
        }
    }
}
