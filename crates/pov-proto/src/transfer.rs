use crc32fast::hash;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ParseError {
    InvalidLength,
    InvalidDownloadFrame,
    ChunkCountExceedsAssembly {
        chunk_count: usize,
        max_chunks: usize,
    },
    PayloadShapeMismatch,
    CrcMismatch {
        expected: u32,
        actual: u32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EncodeError {
    OutputBufferTooSmall,
}

// ---------------------------------------------------------------------------
// Packet model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SpokeCommand {
    DisplayOff,
    NextImage,
    RandomizeDisplay,
    SetActiveSlot {
        slot: u32,
    },
    ClearAllImages,
    RequestStorageStats,
    RequestAdcSample {
        device: AdcDevice,
    },
    SetAdcMonitorSampleRateHz {
        hz: u16,
    },
    SetHybridHallTriggerThreshold {
        threshold: u16,
    },
    SetSensorOffsets {
        hall_offset_0_degrees: f32,
        hall_offset_1_degrees: f32,
        imu_offset_degrees: f32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct StorageStats {
    pub total_bytes: u32,
    pub used_bytes: u32,
    pub free_bytes: u32,
    pub image_count: u32,
    pub active_image_id: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AdcDevice {
    BoardRev,
    HallEffectSensor2,
    BatteryVoltage,
    HallEffectSensor1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AdcSample {
    pub device: AdcDevice,
    pub raw: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SpokeResponse {
    StorageStats(StorageStats),
    AdcSample(AdcSample),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DownloadKind {
    DisplayImage,
    OtaImage,
    Video,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DownloadChunk<'a> {
    pub kind: DownloadKind,
    pub transfer_id: usize,
    pub chunk_index: usize,
    pub chunk_count: usize,
    pub total_len: usize,
    pub crc32: u32,
    pub payload: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct CommandFrame {
    pub transfer_id: usize,
    pub command: SpokeCommand,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ResponseFrame {
    pub transfer_id: usize,
    pub response: SpokeResponse,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Packet<'a> {
    Download(DownloadChunk<'a>),
    Command(CommandFrame),
    Response(ResponseFrame),
}

// ---------------------------------------------------------------------------
// Wire packet (postcard-serializable)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
enum WirePacket<'a> {
    Download {
        kind: DownloadKind,
        transfer_id: usize,
        chunk_index: usize,
        chunk_count: usize,
        total_len: usize,
        crc32: u32,
        #[serde(borrow)]
        payload: &'a [u8],
    },
    Command {
        transfer_id: usize,
        command: SpokeCommand,
    },
    Response {
        transfer_id: usize,
        response: SpokeResponse,
    },
}

fn validate_download(chunk: &DownloadChunk<'_>) -> Result<(), ParseError> {
    if chunk.chunk_count == 0 {
        return Err(ParseError::InvalidDownloadFrame);
    }

    if chunk.chunk_index >= chunk.chunk_count {
        return Err(ParseError::InvalidDownloadFrame);
    }

    if chunk.total_len == 0 || chunk.payload.is_empty() || chunk.payload.len() > chunk.total_len {
        return Err(ParseError::InvalidDownloadFrame);
    }

    Ok(())
}

pub fn parse_packet<'a>(raw: &'a [u8]) -> Result<Packet<'a>, ParseError> {
    let wire =
        postcard::from_bytes::<WirePacket<'a>>(raw).map_err(|_| ParseError::InvalidLength)?;

    match wire {
        WirePacket::Download {
            kind,
            transfer_id,
            chunk_index,
            chunk_count,
            total_len,
            crc32,
            payload,
        } => {
            let packet = DownloadChunk {
                kind,
                transfer_id,
                chunk_index,
                chunk_count,
                total_len,
                crc32,
                payload,
            };
            validate_download(&packet)?;
            Ok(Packet::Download(packet))
        }
        WirePacket::Command {
            transfer_id,
            command,
        } => Ok(Packet::Command(CommandFrame {
            transfer_id,
            command,
        })),
        WirePacket::Response {
            transfer_id,
            response,
        } => Ok(Packet::Response(ResponseFrame {
            transfer_id,
            response,
        })),
    }
}

pub fn encode_packet(packet: Packet<'_>, out: &mut [u8]) -> Result<usize, EncodeError> {
    let wire = match packet {
        Packet::Download(chunk) => {
            validate_download(&chunk).map_err(|_| EncodeError::OutputBufferTooSmall)?;
            WirePacket::Download {
                kind: chunk.kind,
                transfer_id: chunk.transfer_id,
                chunk_index: chunk.chunk_index,
                chunk_count: chunk.chunk_count,
                total_len: chunk.total_len,
                crc32: chunk.crc32,
                payload: chunk.payload,
            }
        }
        Packet::Command(frame) => WirePacket::Command {
            transfer_id: frame.transfer_id,
            command: frame.command,
        },
        Packet::Response(frame) => WirePacket::Response {
            transfer_id: frame.transfer_id,
            response: frame.response,
        },
    };

    let used = postcard::to_slice(&wire, out).map_err(|_| EncodeError::OutputBufferTooSmall)?;
    Ok(used.len())
}

// ---------------------------------------------------------------------------
// Transfer assembly
// ---------------------------------------------------------------------------

/// A fully assembled and CRC-verified image transfer payload.
///
/// Note: this type is retained for compatibility with sender-side code.
/// The receiver-side ([`TransferAssembly`]) no longer emits this type;
/// use [`ChunkResult`] and [`TransferComplete`] instead.
#[derive(Clone, Debug)]
pub struct CompletedTransfer<const MAX_BYTES: usize> {
    pub kind: DownloadKind,
    pub transfer_id: usize,
    pub crc32: u32,
    pub len: usize,
    pub bytes: [u8; MAX_BYTES],
}

impl<const MAX_BYTES: usize> CompletedTransfer<MAX_BYTES> {
    pub fn payload(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// Metadata returned when all chunks of a transfer have been received.
///
/// CRC verification is intentionally deferred to the consumer (e.g. the
/// storage layer), which verifies after writing the data to its backing store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TransferComplete {
    pub kind: DownloadKind,
    pub transfer_id: usize,
    pub expected_crc32: u32,
    pub total_len: usize,
}

/// Result of pushing a single chunk into a [`TransferAssembly`].
#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ChunkResult {
    /// The chunk was accepted and is not the final one.  `byte_offset` is the
    /// chunk's position in the assembled payload (= `chunk_index * MAX_CHUNK_PAYLOAD`).
    Received { byte_offset: usize },
    /// This was the last missing chunk.  The data at `byte_offset` still needs
    /// to be persisted before calling commit.
    ReceivedAndComplete {
        byte_offset: usize,
        complete: TransferComplete,
    },
    /// This chunk index was already seen; no action required.
    Duplicate,
}

/// Accumulates out-of-order chunks for a single in-progress transfer and
/// signals completion when every chunk has been received.
///
/// Unlike the previous design the assembly no longer buffers the full payload
/// in RAM.  Each chunk's payload must be forwarded to persistent storage by
/// the caller as it arrives; the assembly only tracks which indices have been
/// received.
pub struct TransferAssembly<
    const MAX_CHUNK_PAYLOAD: usize,
    const MAX_TRANSFER_BYTES: usize,
    const MAX_CHUNKS: usize,
> {
    kind: DownloadKind,
    transfer_id: usize,
    chunk_count: usize,
    total_len: usize,
    crc32: u32,
    received: [bool; MAX_CHUNKS],
    received_count: usize,
}

impl<const MAX_CHUNK_PAYLOAD: usize, const MAX_TRANSFER_BYTES: usize, const MAX_CHUNKS: usize>
    TransferAssembly<MAX_CHUNK_PAYLOAD, MAX_TRANSFER_BYTES, MAX_CHUNKS>
{
    pub const fn new() -> Self {
        Self {
            kind: DownloadKind::DisplayImage,
            transfer_id: 0,
            chunk_count: 0,
            total_len: 0,
            crc32: 0,
            received: [false; MAX_CHUNKS],
            received_count: 0,
        }
    }

    pub fn is_new_transfer(&self, chunk: &DownloadChunk<'_>) -> bool {
        self.kind != chunk.kind
            || self.transfer_id != chunk.transfer_id
            || self.chunk_count != chunk.chunk_count
            || self.total_len != chunk.total_len
            || self.crc32 != chunk.crc32
    }

    pub fn received_count(&self) -> usize {
        self.received_count
    }

    pub fn chunk_count(&self) -> usize {
        self.chunk_count
    }

    fn reset(&mut self, chunk: &DownloadChunk<'_>) {
        self.kind = chunk.kind;
        self.transfer_id = chunk.transfer_id;
        self.chunk_count = chunk.chunk_count;
        self.total_len = chunk.total_len;
        self.crc32 = chunk.crc32;
        self.received = [false; MAX_CHUNKS];
        self.received_count = 0;
    }

    pub fn push_download(&mut self, chunk: DownloadChunk<'_>) -> Result<ChunkResult, ParseError> {
        validate_download(&chunk)?;

        if chunk.chunk_count > MAX_CHUNKS {
            return Err(ParseError::ChunkCountExceedsAssembly {
                chunk_count: chunk.chunk_count,
                max_chunks: MAX_CHUNKS,
            });
        }

        if chunk.payload.len() > MAX_CHUNK_PAYLOAD || chunk.total_len > MAX_TRANSFER_BYTES {
            return Err(ParseError::PayloadShapeMismatch);
        }

        if self.is_new_transfer(&chunk) {
            self.reset(&chunk);
        }

        let byte_offset = chunk.chunk_index * MAX_CHUNK_PAYLOAD;
        let end = byte_offset + chunk.payload.len();
        if end > self.total_len {
            return Err(ParseError::PayloadShapeMismatch);
        }

        if self.received[chunk.chunk_index] {
            return Ok(ChunkResult::Duplicate);
        }

        self.received[chunk.chunk_index] = true;
        self.received_count = self.received_count.saturating_add(1);

        if self.received_count != self.chunk_count {
            return Ok(ChunkResult::Received { byte_offset });
        }

        // All chunks received — signal completion without CRC check (deferred to storage).
        let complete = TransferComplete {
            kind: self.kind,
            transfer_id: self.transfer_id,
            expected_crc32: self.crc32,
            total_len: self.total_len,
        };

        // Reset tracking for the next transfer.
        self.received_count = 0;
        self.received = [false; MAX_CHUNKS];

        Ok(ChunkResult::ReceivedAndComplete {
            byte_offset,
            complete,
        })
    }
}

impl<const MAX_CHUNK_PAYLOAD: usize, const MAX_TRANSFER_BYTES: usize, const MAX_CHUNKS: usize>
    Default for TransferAssembly<MAX_CHUNK_PAYLOAD, MAX_TRANSFER_BYTES, MAX_CHUNKS>
{
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Chunk iterator (sender side)
// ---------------------------------------------------------------------------

pub struct ChunkIter<'a> {
    payload: &'a [u8],
    kind: DownloadKind,
    transfer_id: usize,
    chunk_count: usize,
    total_len: usize,
    crc32: u32,
    max_chunk_payload: usize,
    next_index: usize,
}

impl<'a> ChunkIter<'a> {
    pub fn new(
        payload: &'a [u8],
        kind: DownloadKind,
        transfer_id: usize,
        max_chunk_payload: usize,
    ) -> Option<Self> {
        if payload.is_empty() || max_chunk_payload == 0 {
            return None;
        }

        let chunk_count = payload.len().div_ceil(max_chunk_payload);

        Some(Self {
            payload,
            kind,
            transfer_id,
            chunk_count,
            total_len: payload.len(),
            crc32: hash(payload),
            max_chunk_payload,
            next_index: 0,
        })
    }
}

impl<'a> Iterator for ChunkIter<'a> {
    type Item = DownloadChunk<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_index >= self.chunk_count {
            return None;
        }

        let chunk_index = self.next_index;
        self.next_index += 1;

        let start = chunk_index * self.max_chunk_payload;
        let end = (start + self.max_chunk_payload).min(self.payload.len());

        Some(DownloadChunk {
            kind: self.kind,
            transfer_id: self.transfer_id,
            chunk_index,
            chunk_count: self.chunk_count,
            total_len: self.total_len,
            crc32: self.crc32,
            payload: &self.payload[start..end],
        })
    }
}
