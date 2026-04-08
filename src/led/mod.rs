mod strip;
#[cfg(feature = "waveshare-matrix")]
mod waveshare_matrix;

pub use strip::{LedError, LedStrip, LedTimings};
#[cfg(feature = "waveshare-matrix")]
pub use waveshare_matrix::WaveshareMatrix;
#[cfg(feature = "waveshare-matrix")]
pub use waveshare_matrix::WaveshareMatrixPins;
