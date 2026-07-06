//! USB-Serial JTAG receiver task.
//!
//! Reads COBS-framed [`pov_proto::bridge::BridgeFrame`] messages from a host
//! workstation and routes each payload directly into the networking ingest
//! pipeline, bypassing the wireless transports entirely.
//!
//! The wire format is identical to what the bridge firmware produces: each
//! message is a [`pov_proto::bridge::BridgeFrame`] serialised with postcard
//! and delimited by a zero byte (COBS framing).  The
//! [`pov_proto::bridge::TransportSelector`] field tells us which assembly
//! buffer to target so that chunk sizes are validated correctly.

use defmt::warn;
use embedded_io_async::Read;
use esp_hal::usb_serial_jtag::UsbSerialJtag;
use pov_proto::bridge::{BridgeFrame, TransportSelector};

/// Maximum on-wire COBS frame size
const RX_BUF: usize = 2048;

/// Embassy task: drain `usb` byte-by-byte, reassemble COBS frames
/// (zero-byte delimited), deserialise each frame as a
/// [`pov_proto::bridge::BridgeFrame`], and dispatch the payload to the
/// correct ingest function based on the [`TransportSelector`].
#[embassy_executor::task]
pub async fn usb_serial_task(mut usb: UsbSerialJtag<'static, esp_hal::Async>) {
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
                    Ok(BridgeFrame::Data {
                        transport, payload, ..
                    }) => match transport {
                        #[cfg(feature = "espnow")]
                        TransportSelector::EspNow => {
                            if super::ingest_espnow_payload(payload, [0u8; 6]).is_err() {
                                warn!("usb_serial: ingest_espnow_payload failed");
                            }
                        }
                        #[allow(unreachable_patterns)]
                        _ => {
                            warn!("usb_serial: dropping frame for unsupported transport");
                        }
                    },
                    Ok(BridgeFrame::ControlRequest(_)) => {}
                    Err(_) => {
                        // Malformed frame — discard silently.
                        warn!("usb_serial: malformed COBS frame, discarding");
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
