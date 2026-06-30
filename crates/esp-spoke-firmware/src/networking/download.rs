use core::cell::RefCell;

use critical_section::Mutex;
use defmt::{info, warn};
use pov_proto::transfer::{
    ChunkResult, CommandFrame, DownloadChunk, DownloadKind, Packet, ParseError, TransferAssembly,
    parse_packet,
};

/// BLE extended advertising limits manufacturer-specific payload to ~250 bytes.
pub const BLE_MAX_CHUNK_PAYLOAD: usize = 224;
/// ESP-NOW 2.0 supports up to 1470-byte packets including pov-proto metadata.
/// Keep chunk payload at a multiple of 4 so flash write offsets are word-aligned.
pub const ESPNOW_MAX_CHUNK_PAYLOAD: usize = 1448;
pub const MAX_TRANSFER_BYTES: usize = 256 * 1024;

#[cfg(feature = "ble")]
const BLE_MAX_CHUNKS: usize = MAX_TRANSFER_BYTES.div_ceil(BLE_MAX_CHUNK_PAYLOAD);
#[cfg(feature = "espnow")]
const ESPNOW_MAX_CHUNKS: usize = MAX_TRANSFER_BYTES.div_ceil(ESPNOW_MAX_CHUNK_PAYLOAD);

pub type IngestError = ParseError;

/// A single received chunk, ready to be streamed to storage.
///
/// `data` is a heap-allocated copy of the raw chunk payload (compressed image
/// bytes at `byte_offset` within the full transfer).  `is_final` is `true`
/// when this chunk completes the transfer; the caller should commit the slot
/// after persisting this chunk.
pub struct NetworkChunk {
    pub transfer_id: usize,
    pub byte_offset: u32,
    pub kind: DownloadKind,
    pub total_len: u32,
    pub expected_crc32: u32,
    pub data: alloc::boxed::Box<[u8]>,
    pub is_final: bool,
}

pub enum IngestedPacket {
    Chunk(NetworkChunk),
    Command(CommandFrame),
}

#[cfg(feature = "ble")]
type BleAssembly = TransferAssembly<BLE_MAX_CHUNK_PAYLOAD, MAX_TRANSFER_BYTES, BLE_MAX_CHUNKS>;
#[cfg(feature = "espnow")]
type EspNowAssembly =
    TransferAssembly<ESPNOW_MAX_CHUNK_PAYLOAD, MAX_TRANSFER_BYTES, ESPNOW_MAX_CHUNKS>;

#[cfg(feature = "ble")]
static BLE_ASSEMBLY: Mutex<RefCell<BleAssembly>> = Mutex::new(RefCell::new(BleAssembly::new()));
#[cfg(feature = "espnow")]
static ESPNOW_ASSEMBLY: Mutex<RefCell<EspNowAssembly>> =
    Mutex::new(RefCell::new(EspNowAssembly::new()));

#[cfg(feature = "ble")]
pub fn ingest_ble_payload(payload: &[u8]) -> Result<Option<IngestedPacket>, IngestError> {
    ingest_packet(payload, &BLE_ASSEMBLY)
}

#[cfg(feature = "espnow")]
pub fn ingest_espnow_payload(payload: &[u8]) -> Result<Option<IngestedPacket>, IngestError> {
    ingest_packet(payload, &ESPNOW_ASSEMBLY)
}

#[cfg(any(feature = "ble", feature = "espnow"))]
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

#[cfg(any(feature = "ble", feature = "espnow"))]
fn ingest_chunk<const MCP: usize, const MC: usize>(
    chunk: DownloadChunk<'_>,
    assembly: &Mutex<RefCell<TransferAssembly<MCP, MAX_TRANSFER_BYTES, MC>>>,
) -> Result<Option<IngestedPacket>, IngestError> {
    let new_transfer = critical_section::with(|cs| assembly.borrow_ref(cs).is_new_transfer(&chunk));

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

    // Capture fields from chunk before it is moved into push_download.
    let transfer_id = chunk.transfer_id;
    let byte_offset = (chunk.chunk_index * MCP) as u32;
    let kind = chunk.kind;
    let total_len = chunk.total_len as u32;
    let expected_crc32 = chunk.crc32;
    // Copy the payload into a heap allocation before the borrow ends.
    let data: alloc::boxed::Box<[u8]> = chunk.payload.into();

    let result = critical_section::with(|cs| assembly.borrow_ref_mut(cs).push_download(chunk));

    match result {
        Ok(ChunkResult::Received { .. }) => {
            let (received, total) = critical_section::with(|cs| {
                let a = assembly.borrow_ref(cs);
                (a.received_count(), a.chunk_count())
            });
            info!(
                "chunk stored: kind={:?} transfer_id={=usize} offset={=u32} have={=usize}/{=usize}",
                kind, transfer_id, byte_offset, received, total,
            );
            Ok(Some(IngestedPacket::Chunk(NetworkChunk {
                transfer_id,
                byte_offset,
                kind,
                total_len,
                expected_crc32,
                data,
                is_final: false,
            })))
        }
        Ok(ChunkResult::ReceivedAndComplete { complete, .. }) => {
            info!(
                "transfer complete: kind={:?} transfer_id={=usize} bytes={=usize} crc32={=u32}",
                complete.kind, complete.transfer_id, complete.total_len, complete.expected_crc32
            );
            Ok(Some(IngestedPacket::Chunk(NetworkChunk {
                transfer_id,
                byte_offset,
                kind,
                total_len,
                expected_crc32,
                data,
                is_final: true,
            })))
        }
        Ok(ChunkResult::Duplicate) => {
            info!(
                "duplicate chunk ignored: transfer_id={=usize} offset={=u32}",
                transfer_id, byte_offset
            );
            Ok(None)
        }
        Err(err) => {
            warn!(
                "chunk rejected: kind={:?} transfer_id={=usize} offset={=u32} reason={:?}",
                kind, transfer_id, byte_offset, err
            );
            Err(err)
        }
    }
}
