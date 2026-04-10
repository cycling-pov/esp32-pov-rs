use serde::{Deserialize, Serialize};

/// Magic bytes that prefix every image wire payload.
pub const MAGIC: [u8; 3] = *b"POV";

/// Wire format version.
pub const WIRE_VERSION: u8 = 1;

/// Fixed byte length of the image payload header (`MAGIC` + version + encoding).
pub const HEADER_LEN: usize = 5;

// ---------------------------------------------------------------------------
// Encoding identifiers
// ---------------------------------------------------------------------------

/// Identifies the pixel encoding and compression scheme in an image payload.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Encoding {
    /// 16-bit RGB565 pixels compressed with zlib (DEFLATE + zlib header/
    /// trailer, matching Python's `zlib.compress()`).
    Rgb565Deflate,
}

/// Wire representation of the image payload header.
#[cfg(any(feature = "image-decode", feature = "image-encode"))]
#[derive(Serialize, Deserialize)]
struct ImageHeader {
    magic: [u8; 3],
    version: u8,
    encoding: Encoding,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ImageWireError {
    MissingHeader,
    InvalidMagic,
    UnsupportedVersion {
        version: u8,
    },
    UnsupportedEncoding {
        encoding: u8,
    },
    /// Scratch buffer is too small to hold the decompressed RGB565 pixels.
    ScratchTooSmall {
        needed: usize,
        actual: usize,
    },
    /// Decompressed RGB565 data length is inconsistent with the output pixel
    /// count.
    InvalidRgb565Length {
        needed: usize,
        actual: usize,
    },
    DeflateDecompressionFailed,
    DeflateOutputTooLarge {
        max: usize,
    },
    /// Output/input buffer is too small to hold the encoded result.
    OutputBufferTooSmall {
        needed: usize,
        actual: usize,
    },
    /// Input RGB888 byte count is not a multiple of 3.
    InvalidRgb888Length {
        len: usize,
    },
}

// ---------------------------------------------------------------------------
// Decode path (feature = "image-decode")
// ---------------------------------------------------------------------------

#[cfg(feature = "image-decode")]
use core::iter;

#[cfg(feature = "image-decode")]
use miniz_oxide::inflate::{TINFLStatus, decompress_slice_iter_to_slice};

#[cfg(feature = "image-decode")]
use rgb::RGB8;

/// Decode a framed image wire payload into RGB8 pixels.
///
/// * `bytes` – raw bytes starting with the 5-byte image header.
/// * `scratch` – caller-supplied temporary buffer; must be at least
///   `output.len() * 2` bytes for RGB565 decoding.
/// * `output` – destination pixel slice; decoded pixels are written here.
/// * `mode` – controls whether the pixel count must match exactly or whether
///   a prefix is acceptable (useful for partial frames on small displays).
#[cfg(feature = "image-decode")]
pub fn decode_into_rgb8(
    bytes: &[u8],
    scratch: &mut [u8],
    output: &mut [RGB8],
    mode: DecodeMode,
) -> Result<Encoding, ImageWireError> {
    if bytes.len() < HEADER_LEN {
        return Err(ImageWireError::MissingHeader);
    }

    let (hdr, image_payload) = postcard::take_from_bytes::<ImageHeader>(bytes)
        .map_err(|_| ImageWireError::UnsupportedEncoding { encoding: bytes[4] })?;

    if hdr.magic != MAGIC {
        return Err(ImageWireError::InvalidMagic);
    }

    if hdr.version != WIRE_VERSION {
        return Err(ImageWireError::UnsupportedVersion {
            version: hdr.version,
        });
    }

    let encoding = hdr.encoding;

    match encoding {
        Encoding::Rgb565Deflate => {
            let max_expected = output.len() * 2;
            if scratch.len() < max_expected {
                return Err(ImageWireError::ScratchTooSmall {
                    needed: max_expected,
                    actual: scratch.len(),
                });
            }

            let decoded_len = match decompress_slice_iter_to_slice(
                &mut scratch[..max_expected],
                iter::once(image_payload),
                true,
                true, // zlib format header – matches Python's zlib.compress()
            ) {
                Ok(n) => n,
                Err(TINFLStatus::HasMoreOutput) if matches!(mode, DecodeMode::PrefixPixels) => {
                    max_expected
                }
                Err(TINFLStatus::HasMoreOutput) => {
                    return Err(ImageWireError::DeflateOutputTooLarge { max: max_expected });
                }
                Err(_) => return Err(ImageWireError::DeflateDecompressionFailed),
            };

            decode_rgb565_to_rgb8(&scratch[..decoded_len], output, mode)?;
            Ok(Encoding::Rgb565Deflate)
        }
    }
}

/// Controls how pixel-count mismatches are handled during decoding.
#[cfg(feature = "image-decode")]
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DecodeMode {
    /// The decompressed pixel count must exactly match `output.len()`.
    ExactPixels,
    /// Fewer pixels than `output.len()` are accepted; only decoded pixels are
    /// written and any remaining output pixels are left unchanged.
    PrefixPixels,
}

#[cfg(feature = "image-decode")]
fn decode_rgb565_to_rgb8(
    encoded: &[u8],
    output: &mut [RGB8],
    mode: DecodeMode,
) -> Result<(), ImageWireError> {
    let needed = output.len() * 2;
    match mode {
        DecodeMode::ExactPixels if encoded.len() != needed => {
            return Err(ImageWireError::InvalidRgb565Length {
                needed,
                actual: encoded.len(),
            });
        }
        _ if encoded.len() < needed => {
            return Err(ImageWireError::InvalidRgb565Length {
                needed,
                actual: encoded.len(),
            });
        }
        _ => {}
    }

    for (pixel, chunk) in output.iter_mut().zip(encoded.chunks_exact(2)) {
        let word = u16::from_le_bytes([chunk[0], chunk[1]]);
        let r5 = ((word >> 11) & 0x1f) as u8;
        let g6 = ((word >> 5) & 0x3f) as u8;
        let b5 = (word & 0x1f) as u8;
        *pixel = RGB8 {
            r: (r5 << 3) | (r5 >> 2),
            g: (g6 << 2) | (g6 >> 4),
            b: (b5 << 3) | (b5 >> 2),
        };
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Encode path (feature = "image-encode")  –  requires alloc
// ---------------------------------------------------------------------------

#[cfg(feature = "image-encode")]
use alloc::vec::Vec;

#[cfg(feature = "image-encode")]
use miniz_oxide::deflate::compress_to_vec_zlib;

/// Encode a raw RGB888 byte slice into a framed image wire payload suitable
/// for chunked transmission.
///
/// The output is `MAGIC || WIRE_VERSION || encoding_byte || compressed_rgb565`.
///
/// Compression uses zlib format (DEFLATE + zlib header/trailer) at level 9,
/// which matches the Python sender's `zlib.compress(..., level=9)`.
///
/// # Errors
///
/// Returns [`ImageWireError::InvalidRgb888Length`] if `rgb888.len()` is not a
/// multiple of 3.
#[cfg(feature = "image-encode")]
pub fn encode_rgb888_to_wire(rgb888: &[u8]) -> Result<Vec<u8>, ImageWireError> {
    if !rgb888.len().is_multiple_of(3) {
        return Err(ImageWireError::InvalidRgb888Length { len: rgb888.len() });
    }

    let rgb565 = rgb888_to_rgb565(rgb888);
    let compressed = compress_to_vec_zlib(&rgb565, 9);

    let hdr = ImageHeader {
        magic: MAGIC,
        version: WIRE_VERSION,
        encoding: Encoding::Rgb565Deflate,
    };
    let mut hdr_buf = [0u8; HEADER_LEN];
    // ImageHeader always serializes to exactly HEADER_LEN bytes.
    postcard::to_slice(&hdr, &mut hdr_buf).map_err(|_| ImageWireError::OutputBufferTooSmall {
        needed: HEADER_LEN,
        actual: 0,
    })?;

    let mut out = Vec::with_capacity(HEADER_LEN + compressed.len());
    out.extend_from_slice(&hdr_buf);
    out.extend_from_slice(&compressed);
    Ok(out)
}

#[cfg(feature = "image-encode")]
fn rgb888_to_rgb565(rgb888: &[u8]) -> Vec<u8> {
    let pixel_count = rgb888.len() / 3;
    let mut out = Vec::with_capacity(pixel_count * 2);
    for chunk in rgb888.chunks_exact(3) {
        let r = chunk[0] as u16;
        let g = chunk[1] as u16;
        let b = chunk[2] as u16;
        let word: u16 = ((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3);
        out.push((word & 0xFF) as u8);
        out.push((word >> 8) as u8);
    }
    out
}
