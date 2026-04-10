//! BLE Extended Advertisement broadcaster task.
//!
//! Receives raw chunk payloads from `usb_serial_task` and broadcasts each one
//! as a non-connectable, non-scannable BLE extended advertisement.

use bt_hci::controller::ExternalController;
use embassy_futures::select::select;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Receiver};
use embassy_time::Duration;
use esp_radio::ble::controller::BleConnector;
use trouble_host::{
    Address,
    prelude::{
        AdStructure, Advertisement, AdvertisementParameters, AdvertisementSet, DefaultPacketPool,
        Host, HostResources,
    },
};

use crate::usb_serial::ChunkMsg;

/// Number of simultaneous connections (we never accept connections, but
/// trouble-host requires a non-zero value).
const CONNECTIONS: usize = 1;
/// Number of L2CAP channels.
const L2CAP_CHANNELS: usize = 1;

/// The concrete controller type produced in `main` and passed to this task.
pub type BleController = ExternalController<BleConnector<'static>, 10>;

/// Individual advertisement packet payload buffer.
///
/// BLE extended advertising supports up to 254 bytes of AD payload.
/// We prepend a 4-byte manufacturer-specific AD header so the remaining
/// 250 bytes accommodate the largest possible chunk (224 payload + 24 header =
/// 248 bytes plus 2 bytes AD-type + length = still ≤ 254).
const ADV_DATA_MAX: usize = 254;

/// Random static address used for the BLE broadcaster role.
const BROADCASTER_ADDR: [u8; 6] = [0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFF];

#[embassy_executor::task]
pub async fn ble_adv_task(
    controller: BleController,
    receiver: Receiver<'static, CriticalSectionRawMutex, ChunkMsg, 4>,
) {
    let mut resources = HostResources::<DefaultPacketPool, CONNECTIONS, L2CAP_CHANNELS>::new();
    let random_addr = Address::random(BROADCASTER_ADDR);
    let builder = trouble_host::new(controller, &mut resources).set_random_address(random_addr);
    let stack = builder.build();

    let Host {
        mut runner,
        mut peripheral,
        ..
    } = stack;

    // The advertising loop: dequeues one chunk at a time and fires a short
    // non-connectable broadcast.  `runner.run()` processes HCI events on the
    // other side of `select`.
    let adv_loop = async {
        loop {
            let msg = receiver.receive().await;

            // Build an AD structure: manufacturer-specific data (type 0xFF).
            // Layout: [len][0xFF][chunk bytes…]
            // We use little-endian company ID 0xFFFF (not a real company; fine
            // for a private/unicast broadcast network).
            let chunk = &msg.buf[..msg.len];
            let mut adv_buf = [0u8; ADV_DATA_MAX];

            // The AdStructure helper encodes the type+length prefix for us.
            let ad = [AdStructure::ManufacturerSpecificData {
                company_identifier: 0xFFFF,
                payload: chunk,
            }];
            let Ok(adv_data_len) = AdStructure::encode_slice(&ad, &mut adv_buf) else {
                // Chunk too large — skip.
                continue;
            };

            let sets = [AdvertisementSet {
                params: AdvertisementParameters {
                    // Each broadcast window is short; we only need one shot per chunk.
                    timeout: Some(Duration::from_millis(200)),
                    interval_min: Duration::from_millis(100),
                    interval_max: Duration::from_millis(100),
                    ..Default::default()
                },
                data: Advertisement::ExtNonconnectableNonscannableUndirected {
                    anonymous: false,
                    adv_data: &adv_buf[..adv_data_len],
                },
            }];

            let mut handles = AdvertisementSet::handles(&sets);

            if let Ok(advertiser) = peripheral.advertise_ext(&sets, &mut handles).await {
                // For a non-connectable advertisement we expect Timeout; any
                // other error is also acceptable to ignore here.
                let _ = advertiser.accept().await;
            }
        }
    };

    select(runner.run(), adv_loop).await;
}
