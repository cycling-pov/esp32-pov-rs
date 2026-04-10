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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SpokeCommand {
    DisplayOff,
    NextImage,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct CommandFrame {
    pub transfer_id: usize,
    pub command: SpokeCommand,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Packet<'a> {
    Download(DownloadChunk<'a>),
    Command(CommandFrame),
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
    };

    let used = postcard::to_slice(&wire, out).map_err(|_| EncodeError::OutputBufferTooSmall)?;
    Ok(used.len())
}

// ---------------------------------------------------------------------------
// Transfer assembly
// ---------------------------------------------------------------------------

/// A fully assembled and CRC-verified image transfer payload.
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

/// Accumulates out-of-order chunks for a single in-progress transfer and
/// yields a [`CompletedTransfer`] when every chunk has been received and the
/// CRC matches.
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
    payload_lengths: [usize; MAX_CHUNKS],
    payload: [u8; MAX_TRANSFER_BYTES],
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
            payload_lengths: [0; MAX_CHUNKS],
            payload: [0; MAX_TRANSFER_BYTES],
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
        self.payload_lengths = [0; MAX_CHUNKS];
        self.payload.fill(0);
    }

    pub fn push_download(
        &mut self,
        chunk: DownloadChunk<'_>,
    ) -> Result<Option<CompletedTransfer<MAX_TRANSFER_BYTES>>, ParseError> {
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

        let start = chunk.chunk_index * MAX_CHUNK_PAYLOAD;
        let end = start + chunk.payload.len();
        if end > self.total_len || end > self.payload.len() {
            return Err(ParseError::PayloadShapeMismatch);
        }

        self.payload[start..end].copy_from_slice(chunk.payload);

        if !self.received[chunk.chunk_index] {
            self.received[chunk.chunk_index] = true;
            self.received_count = self.received_count.saturating_add(1);
        }

        self.payload_lengths[chunk.chunk_index] = chunk.payload.len();

        if self.received_count != self.chunk_count {
            return Ok(None);
        }

        if !self.is_payload_shape_valid() {
            return Err(ParseError::PayloadShapeMismatch);
        }

        let actual_crc = hash(&self.payload[..self.total_len]);
        if actual_crc != self.crc32 {
            return Err(ParseError::CrcMismatch {
                expected: self.crc32,
                actual: actual_crc,
            });
        }

        let mut bytes = [0u8; MAX_TRANSFER_BYTES];
        bytes[..self.total_len].copy_from_slice(&self.payload[..self.total_len]);
        let completed = CompletedTransfer {
            kind: self.kind,
            transfer_id: self.transfer_id,
            crc32: self.crc32,
            len: self.total_len,
            bytes,
        };

        self.received_count = 0;
        self.received = [false; MAX_CHUNKS];

        Ok(Some(completed))
    }

    fn is_payload_shape_valid(&self) -> bool {
        if self.chunk_count == 0 {
            return false;
        }

        for index in 0..self.chunk_count.saturating_sub(1) {
            if self.payload_lengths[index] != MAX_CHUNK_PAYLOAD {
                return false;
            }
        }

        let tail_len = self.total_len % MAX_CHUNK_PAYLOAD;
        let expected_last_len = if tail_len == 0 {
            MAX_CHUNK_PAYLOAD
        } else {
            tail_len
        };

        self.payload_lengths[self.chunk_count - 1] == expected_last_len
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
