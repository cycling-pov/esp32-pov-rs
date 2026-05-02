use serde::{Deserialize, Serialize};

/// Magic bytes that prefix every image wire payload.
pub const MAGIC: [u8; 3] = *b"POV";

/// Wire format version.
pub const WIRE_VERSION: u8 = 1;

/// Minimum byte length of the image payload header (`MAGIC` + version + encoding
/// discriminant).  Variable-length encoding variants may produce headers larger
/// than this; the full header is always parsed via `postcard::take_from_bytes`.
pub const HEADER_LEN: usize = 5;

// ---------------------------------------------------------------------------
// Encoding identifiers
// ---------------------------------------------------------------------------

/// Identifies the pixel encoding and compression scheme in an image payload.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Encoding {
    /// 24-bit RGB888 pixels laid out in row-major Cartesian (x, y) order,
    /// compressed with zlib (DEFLATE + zlib header/trailer, matching Python's
    /// `zlib.compress()`).
    Rgb888Deflate,
    /// 24-bit RGB888 pixels laid out in polar order: `pixels[radial][led]`,
    /// where `radial` is the angular position index (0 = 0°, `radials - 1` ≈
    /// 360°) and `led` is the LED index along the spoke (0 = centre).
    /// Compressed with zlib, same as `Rgb888Deflate`.
    PolarRgb888Deflate {
        /// Number of LEDs per radial strip (spoke length).
        leds: u8,
        /// Number of angular positions (radial strips) in the image.
        radials: u16,
    },
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
    /// Scratch buffer is too small to hold the decompressed pixels.
    ScratchTooSmall {
        needed: usize,
        actual: usize,
    },
    /// Decompressed data length is inconsistent with the output pixel count.
    InvalidDecompressedLength {
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
///   `output.len() * 3` bytes for RGB888 decoding.
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
        Encoding::Rgb888Deflate => {
            let max_expected = output.len() * 3;
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

            decode_rgb888_to_rgb8(&scratch[..decoded_len], output, mode)?;
            Ok(Encoding::Rgb888Deflate)
        }
        Encoding::PolarRgb888Deflate { leds, radials } => {
            let pixel_count = leds as usize * radials as usize;
            let max_raw = pixel_count * 3;
            if scratch.len() < max_raw {
                return Err(ImageWireError::ScratchTooSmall {
                    needed: max_raw,
                    actual: scratch.len(),
                });
            }
            if output.len() != pixel_count {
                return Err(ImageWireError::InvalidDecompressedLength {
                    needed: pixel_count,
                    actual: output.len(),
                });
            }

            let decoded_len = match decompress_slice_iter_to_slice(
                &mut scratch[..max_raw],
                iter::once(image_payload),
                true,
                true,
            ) {
                Ok(n) => n,
                Err(TINFLStatus::HasMoreOutput) => {
                    return Err(ImageWireError::DeflateOutputTooLarge { max: max_raw });
                }
                Err(_) => return Err(ImageWireError::DeflateDecompressionFailed),
            };

            decode_rgb888_to_rgb8(&scratch[..decoded_len], output, DecodeMode::ExactPixels)?;
            Ok(Encoding::PolarRgb888Deflate { leds, radials })
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
fn decode_rgb888_to_rgb8(
    encoded: &[u8],
    output: &mut [RGB8],
    mode: DecodeMode,
) -> Result<(), ImageWireError> {
    let needed = output.len() * 3;
    match mode {
        DecodeMode::ExactPixels if encoded.len() != needed => {
            return Err(ImageWireError::InvalidDecompressedLength {
                needed,
                actual: encoded.len(),
            });
        }
        _ if encoded.len() < needed => {
            return Err(ImageWireError::InvalidDecompressedLength {
                needed,
                actual: encoded.len(),
            });
        }
        _ => {}
    }

    for (pixel, chunk) in output.iter_mut().zip(encoded.chunks_exact(3)) {
        *pixel = RGB8 {
            r: chunk[0],
            g: chunk[1],
            b: chunk[2],
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
/// The output is `MAGIC || WIRE_VERSION || encoding_byte || compressed_rgb888`.
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

    let compressed = compress_to_vec_zlib(rgb888, 9);

    let hdr = ImageHeader {
        magic: MAGIC,
        version: WIRE_VERSION,
        encoding: Encoding::Rgb888Deflate,
    };
    let mut hdr_buf = [0u8; HEADER_LEN];
    // Rgb888Deflate header serializes to exactly HEADER_LEN bytes.
    postcard::to_slice(&hdr, &mut hdr_buf).map_err(|_| ImageWireError::OutputBufferTooSmall {
        needed: HEADER_LEN,
        actual: 0,
    })?;

    let mut out = Vec::with_capacity(HEADER_LEN + compressed.len());
    out.extend_from_slice(&hdr_buf);
    out.extend_from_slice(&compressed);
    Ok(out)
}

/// Encode polar RGB888 pixels into a framed image wire payload.
///
/// `pixels` must be `leds * radials * 3` bytes laid out as
/// `pixels[radial * leds * 3 + led * 3 ..]` (radial-major, RGB888 per LED).
///
/// Compression uses zlib format (DEFLATE + zlib header/trailer) at level 9.
#[cfg(feature = "image-encode")]
pub fn encode_polar_rgb888_to_wire(
    pixels: &[u8],
    leds: u8,
    radials: u16,
) -> Result<Vec<u8>, ImageWireError> {
    let expected_len = leds as usize * radials as usize * 3;
    if pixels.len() != expected_len {
        return Err(ImageWireError::InvalidRgb888Length { len: pixels.len() });
    }

    let compressed = compress_to_vec_zlib(pixels, 9);

    let hdr = ImageHeader {
        magic: MAGIC,
        version: WIRE_VERSION,
        encoding: Encoding::PolarRgb888Deflate { leds, radials },
    };
    // PolarRgb888Deflate header is larger than HEADER_LEN; use a 16-byte buffer.
    let mut hdr_buf = [0u8; 16];
    let hdr_bytes = postcard::to_slice(&hdr, &mut hdr_buf).map_err(|_| {
        ImageWireError::OutputBufferTooSmall {
            needed: 16,
            actual: 0,
        }
    })?;

    let mut out = Vec::with_capacity(hdr_bytes.len() + compressed.len());
    out.extend_from_slice(hdr_bytes);
    out.extend_from_slice(&compressed);
    Ok(out)
}
