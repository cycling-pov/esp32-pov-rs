//! ESP-NOW task.
//!
//! Handles host->spoke sends and listens for inbound discovery beacons so the
//! host can list/select available peers for targeted sends.

use core::cell::RefCell;

use defmt::{debug, info, warn};
use embassy_futures::select::{Either, select};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::{Receiver, Sender},
};
use esp_radio::esp_now::{BROADCAST_ADDRESS, EspNow, EspNowWifiInterface, PeerInfo};
use pov_proto::bridge::{ESPNOW_DISCOVERY_BEACON, EspNowPeerList, EspNowTarget, MAX_ESP_NOW_PEERS};

use crate::usb_serial::ChunkMsg;

const TRACKED_PEERS: usize = MAX_ESP_NOW_PEERS;

#[derive(Clone, Copy)]
struct PeerTable {
    peers: [[u8; 6]; TRACKED_PEERS],
    count: usize,
}

impl PeerTable {
    const fn new() -> Self {
        Self {
            peers: [[0; 6]; TRACKED_PEERS],
            count: 0,
        }
    }

    fn add(&mut self, peer: [u8; 6]) {
        if self.peers[..self.count].contains(&peer) {
            return;
        }

        if self.count < TRACKED_PEERS {
            self.peers[self.count] = peer;
            self.count += 1;
            return;
        }

        // Keep most recently seen peers by rotating out the oldest one.
        self.peers.rotate_left(1);
        self.peers[self.count - 1] = peer;
    }

    fn snapshot(&self) -> EspNowPeerList {
        let mut out = EspNowPeerList::empty();
        out.count = self.count as u8;
        out.peers[..self.count].copy_from_slice(&self.peers[..self.count]);
        out
    }
}

static PEER_TABLE: critical_section::Mutex<RefCell<PeerTable>> =
    critical_section::Mutex::new(RefCell::new(PeerTable::new()));

#[derive(Clone, Copy)]
pub struct InboundMsg {
    pub src: [u8; 6],
    pub len: usize,
    pub buf: [u8; 1470],
}

pub fn snapshot_peers() -> EspNowPeerList {
    critical_section::with(|cs| PEER_TABLE.borrow_ref(cs).snapshot())
}

fn record_peer(peer: [u8; 6]) {
    critical_section::with(|cs| {
        PEER_TABLE.borrow_ref_mut(cs).add(peer);
    });
}

#[embassy_executor::task]
pub async fn esp_now_task(
    mut esp_now: EspNow<'static>,
    receiver: Receiver<'static, CriticalSectionRawMutex, ChunkMsg, 4>,
    inbound_tx: Sender<'static, CriticalSectionRawMutex, InboundMsg, 4>,
) {
    info!("ESP-NOW task started");

    loop {
        match select(receiver.receive(), esp_now.receive_async()).await {
            Either::First(msg) => {
                let destination = match msg.esp_now_target {
                    EspNowTarget::Broadcast => BROADCAST_ADDRESS,
                    EspNowTarget::Peer(peer) => peer,
                };

                if let EspNowTarget::Peer(peer) = msg.esp_now_target {
                    if !esp_now.peer_exists(&peer) {
                        if let Err(err) = esp_now.add_peer(PeerInfo {
                            interface: EspNowWifiInterface::Station,
                            peer_address: peer,
                            lmk: None,
                            channel: None,
                            encrypt: false,
                        }) {
                            warn!("Failed to add ESP-NOW peer before send: {:?}", err);
                        }
                    }
                }

                let attempts = usize::from(msg.esp_now_retries).saturating_add(1);
                let mut sent_ok = false;
                for _ in 0..attempts {
                    if esp_now
                        .send_async(&destination, &msg.buf[..msg.len])
                        .await
                        .is_ok()
                    {
                        sent_ok = true;
                        break;
                    }
                }

                if !sent_ok {
                    warn!("ESP-NOW send failed after retries");
                }
            }
            Either::Second(received) => {
                let src = received.info.src_address;
                let payload = received.data();
                record_peer(src);

                if payload == ESPNOW_DISCOVERY_BEACON {
                    debug!(
                        "ESP-NOW discovery beacon from {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                        src[0], src[1], src[2], src[3], src[4], src[5]
                    );
                    continue;
                }

                let mut msg = InboundMsg {
                    src,
                    len: payload.len().min(1470),
                    buf: [0u8; 1470],
                };
                msg.buf[..msg.len].copy_from_slice(&payload[..msg.len]);
                inbound_tx.send(msg).await;
            }
        }
    }
}
