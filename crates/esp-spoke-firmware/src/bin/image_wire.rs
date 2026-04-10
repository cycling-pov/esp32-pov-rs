// Re-export the image wire types and decoder from the shared pov-proto crate
// so that the rest of the binary can use a stable local path.
pub use pov_proto::image::{DecodeMode, decode_into_rgb8};
