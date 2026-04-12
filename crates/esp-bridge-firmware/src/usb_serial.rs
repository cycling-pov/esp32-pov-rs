//! USB-Serial JTAG receiver task.
//!
//! Reads COBS-framed [`pov_proto::bridge::BridgeFrame`] messages from the host
//! workstation and forwards each chunk payload to the appropriate channel
//! depending on the requested transport.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Sender};
use embedded_io_async::Read;
use esp_hal::usb_serial_jtag::UsbSerialJtag;

use pov_proto::bridge::{BridgeFrame, TransportSelector};

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
}

/// Embassy task: drain `usb` byte-by-byte, reassemble COBS frames (zero-byte
/// delimited), deserialise each frame as a [`BridgeFrame`], and dispatch the
/// contained payload to the right channel.
#[embassy_executor::task]
pub async fn usb_serial_task(
    mut usb: UsbSerialJtag<'static, esp_hal::Async>,
    ble_tx: Sender<'static, CriticalSectionRawMutex, ChunkMsg, 4>,
    esp_now_tx: Sender<'static, CriticalSectionRawMutex, ChunkMsg, 4>,
) {
    let mut buf = [0u8; RX_BUF];
    let mut head = 0usize;

    loop {
        // Read one byte at a time to keep the logic simple.
        let mut byte = [0u8; 1];
        if usb.read(&mut byte).await.is_err() {
            continue;
        }
        let b = byte[0];

        if b == 0 {
            // Zero byte = COBS frame delimiter.  Decode the accumulated slice.
            if head > 0 {
                // postcard::from_bytes_cobs mutates the slice in-place.
                match postcard::from_bytes_cobs::<BridgeFrame<'_>>(&mut buf[..head]) {
                    Ok(frame) => {
                        // Copy the borrowed payload before we release `buf`.
                        let mut msg = ChunkMsg {
                            buf: [0u8; 1470],
                            len: 0,
                        };
                        let copy_len = frame.payload.len().min(msg.buf.len());
                        msg.buf[..copy_len].copy_from_slice(&frame.payload[..copy_len]);
                        msg.len = copy_len;

                        match frame.transport {
                            TransportSelector::BleExtAdv => {
                                ble_tx.send(msg).await;
                            }
                            TransportSelector::EspNow => {
                                esp_now_tx.send(msg).await;
                            }
                        }
                    }
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
