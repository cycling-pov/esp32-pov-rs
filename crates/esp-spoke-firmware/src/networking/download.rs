use core::cell::RefCell;

use critical_section::Mutex;
use defmt::{info, warn};
use pov_proto::transfer::{
    CommandFrame, CompletedTransfer, Packet, ParseError, TransferAssembly, parse_packet,
};

pub const MAX_CHUNK_PAYLOAD: usize = 224;
pub const MAX_TRANSFER_BYTES: usize = 10 * 1024;
const MAX_CHUNKS: usize = MAX_TRANSFER_BYTES.div_ceil(MAX_CHUNK_PAYLOAD);

pub type IngestError = ParseError;

pub enum IngestedPacket {
    Download(alloc::boxed::Box<CompletedDownload>),
    Command(CommandFrame),
}

pub type CompletedDownload = CompletedTransfer<MAX_TRANSFER_BYTES>;

type DownloadAssembly = TransferAssembly<MAX_CHUNK_PAYLOAD, MAX_TRANSFER_BYTES, MAX_CHUNKS>;

static DOWNLOAD_ASSEMBLY: Mutex<RefCell<DownloadAssembly>> =
    Mutex::new(RefCell::new(DownloadAssembly::new()));

pub fn ingest_manufacturer_data(payload: &[u8]) -> Result<Option<IngestedPacket>, IngestError> {
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

    let new_transfer =
        critical_section::with(|cs| DOWNLOAD_ASSEMBLY.borrow_ref(cs).is_new_transfer(&chunk));

    if new_transfer {
        if critical_section::with(|cs| DOWNLOAD_ASSEMBLY.borrow_ref(cs).received_count()) == 0 {
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
        critical_section::with(|cs| DOWNLOAD_ASSEMBLY.borrow_ref_mut(cs).push_download(chunk));

    match result {
        Ok(Some(completed)) => {
            info!(
                "transfer complete: kind={:?} transfer_id={=usize} bytes={=usize} crc32={=u32}",
                completed.kind, completed.transfer_id, completed.len, completed.crc32
            );
            Ok(Some(IngestedPacket::Download(alloc::boxed::Box::new(completed))))
        }
        Ok(None) => {
            let (received, total) = critical_section::with(|cs| {
                let a = DOWNLOAD_ASSEMBLY.borrow_ref(cs);
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
