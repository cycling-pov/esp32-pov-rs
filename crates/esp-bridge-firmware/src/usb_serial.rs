//! USB-Serial JTAG receiver task.
//!
//! Reads COBS-framed [`pov_proto::bridge::BridgeFrame`] messages from the host
//! workstation and forwards each chunk payload to the appropriate channel
//! depending on the requested transport.

use embassy_futures::select::{Either, select};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::{Receiver, Sender},
};
use embedded_io_async::Read;
use esp_hal::usb_serial_jtag::UsbSerialJtag;

use pov_proto::bridge::{
    BridgeControlRequest, BridgeControlResponse, BridgeFrame, EspNowTarget, TransportSelector,
};

use crate::esp_now_broadcaster::{InboundMsg, snapshot_peers};

/// Maximum on-wire COBS frame size we are willing to buffer.
///
/// A single BridgeFrame wraps at most 1470 bytes of ESP-NOW 2.0 chunk data
/// plus postcard/COBS framing overhead, 2048 bytes is sufficient.
const RX_BUF: usize = 2048;

/// A fixed-size copy of a raw chunk payload received from the host.
///
/// The payload is opaque from the bridge's perspective; it is written straight
/// into the BLE advertisement or ESP-NOW packet.
/// Sized for the ESP-NOW 2.0 maximum of 1470 bytes per packet.
pub struct ChunkMsg {
    pub buf: [u8; 1470],
    pub len: usize,
    pub esp_now_target: EspNowTarget,
    pub esp_now_retries: u8,
}

async fn reply_control(
    usb: &mut UsbSerialJtag<'static, esp_hal::Async>,
    response: BridgeControlResponse<'_>,
) {
    let mut out = [0u8; 1800];
    if let Ok(cobs) = postcard::to_slice_cobs(&response, &mut out) {
        let _ = usb.write(cobs);
    }
}

/// Embassy task: drain `usb` byte-by-byte, reassemble COBS frames (zero-byte
/// delimited), deserialise each frame as a [`BridgeFrame`], and dispatch the
/// contained payload to the right channel.
#[embassy_executor::task]
pub async fn usb_serial_task(
    mut usb: UsbSerialJtag<'static, esp_hal::Async>,
    ble_tx: Sender<'static, CriticalSectionRawMutex, ChunkMsg, 4>,
    esp_now_tx: Sender<'static, CriticalSectionRawMutex, ChunkMsg, 4>,
    esp_now_inbound_rx: Receiver<'static, CriticalSectionRawMutex, InboundMsg, 4>,
) {
    let mut buf = [0u8; RX_BUF];
    let mut head = 0usize;

    loop {
        let mut byte = [0u8; 1];
        let b = match select(esp_now_inbound_rx.receive(), usb.read(&mut byte)).await {
            Either::First(inbound) => {
                let len = inbound.len.min(inbound.buf.len());
                reply_control(
                    &mut usb,
                    BridgeControlResponse::EspNowInboundPacket {
                        src: inbound.src,
                        payload: &inbound.buf[..len],
                    },
                )
                .await;
                continue;
            }
            Either::Second(Ok(n)) if n == 1 => byte[0],
            _ => continue,
        };

        if b == 0 {
            // Zero byte = COBS frame delimiter.  Decode the accumulated slice.
            if head > 0 {
                // postcard::from_bytes_cobs mutates the slice in-place.
                match postcard::from_bytes_cobs::<BridgeFrame<'_>>(&mut buf[..head]) {
                    Ok(BridgeFrame::Data {
                        transport,
                        esp_now_target,
                        esp_now_retries,
                        payload,
                    }) => {
                        // Copy the borrowed payload before we release `buf`.
                        let mut msg = ChunkMsg {
                            buf: [0u8; 1470],
                            len: 0,
                            esp_now_target,
                            esp_now_retries,
                        };
                        let copy_len = payload.len().min(msg.buf.len());
                        msg.buf[..copy_len].copy_from_slice(&payload[..copy_len]);
                        msg.len = copy_len;

                        match transport {
                            TransportSelector::BleExtAdv => {
                                ble_tx.send(msg).await;
                            }
                            TransportSelector::EspNow => {
                                esp_now_tx.send(msg).await;
                            }
                        }
                    }
                    Ok(BridgeFrame::ControlRequest(req)) => match req {
                        BridgeControlRequest::ListEspNowPeers => {
                            let peers = snapshot_peers();
                            reply_control(&mut usb, BridgeControlResponse::EspNowPeers(peers))
                                .await;
                        }
                    },
                    Err(_) => {
                        // Malformed frame — discard silently.
                    }
                }
            }
            head = 0;
        } else if head < buf.len() {
            buf[head] = b;
            head += 1;
        } else {
            // Buffer overflow — drop the frame and reset.
            head = 0;
        }
    }
}
