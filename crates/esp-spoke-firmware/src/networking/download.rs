use core::cell::RefCell;

use critical_section::Mutex;
use defmt::{info, warn};
use pov_proto::transfer::{
    CommandFrame, CompletedTransfer, DownloadChunk, Packet, ParseError, TransferAssembly,
    parse_packet,
};

/// BLE extended advertising limits manufacturer-specific payload to ~250 bytes.
pub const BLE_MAX_CHUNK_PAYLOAD: usize = 224;
/// ESP-NOW 2.0 supports up to 1470-byte packets including pov-proto metadata.
/// Keep chunk payload below the transport MTU so postcard framing fits too.
pub const ESPNOW_MAX_CHUNK_PAYLOAD: usize = 1450;
pub const MAX_TRANSFER_BYTES: usize = 10 * 1024;

const BLE_MAX_CHUNKS: usize = MAX_TRANSFER_BYTES.div_ceil(BLE_MAX_CHUNK_PAYLOAD);
const ESPNOW_MAX_CHUNKS: usize = MAX_TRANSFER_BYTES.div_ceil(ESPNOW_MAX_CHUNK_PAYLOAD);

pub type IngestError = ParseError;

pub enum IngestedPacket {
    Download(alloc::boxed::Box<CompletedDownload>),
    Command(CommandFrame),
}

pub type CompletedDownload = CompletedTransfer<MAX_TRANSFER_BYTES>;

type BleAssembly = TransferAssembly<BLE_MAX_CHUNK_PAYLOAD, MAX_TRANSFER_BYTES, BLE_MAX_CHUNKS>;
type EspNowAssembly =
    TransferAssembly<ESPNOW_MAX_CHUNK_PAYLOAD, MAX_TRANSFER_BYTES, ESPNOW_MAX_CHUNKS>;

static BLE_ASSEMBLY: Mutex<RefCell<BleAssembly>> =
    Mutex::new(RefCell::new(BleAssembly::new()));
static ESPNOW_ASSEMBLY: Mutex<RefCell<EspNowAssembly>> =
    Mutex::new(RefCell::new(EspNowAssembly::new()));

pub fn ingest_ble_payload(payload: &[u8]) -> Result<Option<IngestedPacket>, IngestError> {
    ingest_packet(payload, &BLE_ASSEMBLY)
}

pub fn ingest_espnow_payload(payload: &[u8]) -> Result<Option<IngestedPacket>, IngestError> {
    ingest_packet(payload, &ESPNOW_ASSEMBLY)
}

fn ingest_packet<const MCP: usize, const MC: usize>(
    payload: &[u8],
    assembly: &Mutex<RefCell<TransferAssembly<MCP, MAX_TRANSFER_BYTES, MC>>>,
) -> Result<Option<IngestedPacket>, IngestError> {
    let packet = match parse_packet(payload) {
        Ok(parsed) => parsed,
        Err(err) => {
            warn!("packet parse failed: {:?}", err);
            return Err(err);
        }
    };

    let chunk = match packet {
        Packet::Download(chunk) => chunk,
        Packet::Command(frame) => return Ok(Some(IngestedPacket::Command(frame))),
    };

    ingest_chunk(chunk, assembly)
}

fn ingest_chunk<const MCP: usize, const MC: usize>(
    chunk: DownloadChunk<'_>,
    assembly: &Mutex<RefCell<TransferAssembly<MCP, MAX_TRANSFER_BYTES, MC>>>,
) -> Result<Option<IngestedPacket>, IngestError> {
    let new_transfer =
        critical_section::with(|cs| assembly.borrow_ref(cs).is_new_transfer(&chunk));

    if new_transfer {
        if critical_section::with(|cs| assembly.borrow_ref(cs).received_count()) == 0 {
            info!(
                "transfer started: kind={:?} transfer_id={=usize} chunks={=usize} bytes={=usize} crc32={=u32}",
                chunk.kind, chunk.transfer_id, chunk.chunk_count, chunk.total_len, chunk.crc32
            );
        } else {
            warn!(
                "transfer metadata changed; resetting: kind={:?} transfer_id={=usize} chunks={=usize} bytes={=usize} crc32={=u32}",
                chunk.kind, chunk.transfer_id, chunk.chunk_count, chunk.total_len, chunk.crc32
            );
        }
    }

    let result =
        critical_section::with(|cs| assembly.borrow_ref_mut(cs).push_download(chunk));

    match result {
        Ok(Some(completed)) => {
            info!(
                "transfer complete: kind={:?} transfer_id={=usize} bytes={=usize} crc32={=u32}",
                completed.kind, completed.transfer_id, completed.len, completed.crc32
            );
            Ok(Some(IngestedPacket::Download(alloc::boxed::Box::new(
                completed,
            ))))
        }
        Ok(None) => {
            let (received, total) = critical_section::with(|cs| {
                let a = assembly.borrow_ref(cs);
                (a.received_count(), a.chunk_count())
            });
            info!(
                "chunk stored: kind={:?} transfer_id={=usize} chunk={=usize}/{=usize} have={=usize}/{=usize}",
                chunk.kind,
                chunk.transfer_id,
                chunk.chunk_index + 1,
                chunk.chunk_count,
                received,
                total,
            );
            Ok(None)
        }
        Err(err) => {
            warn!(
                "chunk rejected: kind={:?} transfer_id={=usize} chunk={=usize}/{=usize} reason={:?}",
                chunk.kind,
                chunk.transfer_id,
                chunk.chunk_index + 1,
                chunk.chunk_count,
                err
            );
            Err(err)
        }
    }
}
