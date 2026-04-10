use serde::{Deserialize, Serialize};

/// Selects which radio transport the bridge should use for a given payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TransportSelector {
    /// BLE 5 Extended Advertising (non-connectable, non-scannable undirected).
    BleExtAdv,
    /// ESP-NOW broadcast to FF:FF:FF:FF:FF:FF.
    EspNow,
}

/// Framing type sent from workstation CLI to wireless bridge over USB Serial JTAG.
///
/// Serialized with postcard COBS framing (zero-byte delimiter).
/// The `payload` field carries a complete encoded pov-proto chunk.
#[derive(Serialize, Deserialize)]
pub struct BridgeFrame<'a> {
    /// Which radio transport the bridge should forward the payload on.
    pub transport: TransportSelector,
    /// The encoded pov-proto chunk bytes to forward verbatim.
    #[serde(borrow)]
    pub payload: &'a [u8],
}
