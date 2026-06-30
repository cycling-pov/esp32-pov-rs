/// Magic bytes that prefix every video wire payload.
pub const MAGIC: [u8; 3] = *b"PVV";

/// Wire format version.
pub const WIRE_VERSION: u8 = 1;

/// Header length in bytes.
pub const HEADER_LEN: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct VideoHeader {
    pub frame_delay_ms: u16,
    pub frame_count: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum VideoWireError {
    MissingHeader,
    InvalidMagic,
    UnsupportedVersion { version: u8 },
    InvalidFrameCount,
    FrameIndexOutOfRange { index: u16, frame_count: u16 },
    TruncatedFrameTable,
    TruncatedFrameData,
}

pub fn parse_header(bytes: &[u8]) -> Result<VideoHeader, VideoWireError> {
    if bytes.len() < HEADER_LEN {
        return Err(VideoWireError::MissingHeader);
    }

    if bytes[..3] != MAGIC {
        return Err(VideoWireError::InvalidMagic);
    }

    let version = bytes[3];
    if version != WIRE_VERSION {
        return Err(VideoWireError::UnsupportedVersion { version });
    }

    let frame_delay_ms = u16::from_le_bytes([bytes[4], bytes[5]]);
    let frame_count = u16::from_le_bytes([bytes[6], bytes[7]]);
    if frame_count == 0 {
        return Err(VideoWireError::InvalidFrameCount);
    }

    Ok(VideoHeader {
        frame_delay_ms,
        frame_count,
    })
}

pub fn frame_at(bytes: &[u8], index: u16) -> Result<&[u8], VideoWireError> {
    let header = parse_header(bytes)?;
    if index >= header.frame_count {
        return Err(VideoWireError::FrameIndexOutOfRange {
            index,
            frame_count: header.frame_count,
        });
    }

    let mut cursor = HEADER_LEN;
    for current in 0..header.frame_count {
        if cursor + 4 > bytes.len() {
            return Err(VideoWireError::TruncatedFrameTable);
        }

        let frame_len = u32::from_le_bytes([
            bytes[cursor],
            bytes[cursor + 1],
            bytes[cursor + 2],
            bytes[cursor + 3],
        ]) as usize;
        cursor += 4;

        if cursor + frame_len > bytes.len() {
            return Err(VideoWireError::TruncatedFrameData);
        }

        if current == index {
            return Ok(&bytes[cursor..cursor + frame_len]);
        }

        cursor += frame_len;
    }

    Err(VideoWireError::FrameIndexOutOfRange {
        index,
        frame_count: header.frame_count,
    })
}
