#[cfg(feature = "sk9822-strip")]
mod sk9822_strip;
mod strip;
#[cfg(feature = "waveshare-matrix")]
mod waveshare_matrix;

#[cfg(feature = "sk9822-strip")]
pub use sk9822_strip::{Sk9822Pins, Sk9822Strip};
pub use strip::{LedError, LedStrip, LedTimings};
#[cfg(feature = "waveshare-matrix")]
pub use waveshare_matrix::WaveshareMatrix;
#[cfg(feature = "waveshare-matrix")]
pub use waveshare_matrix::WaveshareMatrixPins;
