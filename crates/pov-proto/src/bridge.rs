use serde::{Deserialize, Serialize};

pub const MAX_ESP_NOW_PEERS: usize = 32;

/// Discovery beacon sent by spoke firmware over ESP-NOW broadcast.
pub const ESPNOW_DISCOVERY_BEACON: &[u8] = b"POV:DISCOVERY:V1";

/// Selects which radio transport the bridge should use for a given payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TransportSelector {
    /// BLE 5 Extended Advertising (non-connectable, non-scannable undirected).
    BleExtAdv,
    /// ESP-NOW broadcast to FF:FF:FF:FF:FF:FF.
    EspNow,
}

/// Selects which ESP-NOW destination the bridge should use.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EspNowTarget {
    /// Send to FF:FF:FF:FF:FF:FF.
    Broadcast,
    /// Send to a single destination MAC.
    Peer([u8; 6]),
}

/// Host request messages handled by bridge firmware over USB.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum BridgeControlRequest {
    /// Return currently tracked ESP-NOW peers discovered by the bridge.
    ListEspNowPeers,
}

/// Fixed-capacity list of ESP-NOW peers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct EspNowPeerList {
    pub count: u8,
    pub peers: [[u8; 6]; MAX_ESP_NOW_PEERS],
}

impl EspNowPeerList {
    pub const fn empty() -> Self {
        Self {
            count: 0,
            peers: [[0; 6]; MAX_ESP_NOW_PEERS],
        }
    }
}

/// Bridge responses delivered back to host over USB serial.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum BridgeControlResponse {
    EspNowPeers(EspNowPeerList),
}

/// Framing type sent from workstation CLI to wireless bridge over USB Serial JTAG.
///
/// Serialized with postcard COBS framing (zero-byte delimiter).
/// The `payload` field carries a complete encoded pov-proto chunk.
#[derive(Serialize, Deserialize)]
pub enum BridgeFrame<'a> {
    /// Data path: bridge forwards encoded chunk payload over selected radio.
    Data {
        /// Which radio transport the bridge should forward the payload on.
        transport: TransportSelector,
        /// ESP-NOW destination selector (ignored for BLE).
        esp_now_target: EspNowTarget,
        /// Additional retry attempts for ESP-NOW send failures.
        /// 0 means one send attempt.
        esp_now_retries: u8,
        /// The encoded pov-proto chunk bytes to forward verbatim.
        #[serde(borrow)]
        payload: &'a [u8],
    },
    /// Control path handled directly by bridge firmware.
    ControlRequest(BridgeControlRequest),
}

impl<'a> BridgeFrame<'a> {
    pub fn data(
        transport: TransportSelector,
        esp_now_target: EspNowTarget,
        esp_now_retries: u8,
        payload: &'a [u8],
    ) -> Self {
        Self::Data {
            transport,
            esp_now_target,
            esp_now_retries,
            payload,
        }
    }
}
