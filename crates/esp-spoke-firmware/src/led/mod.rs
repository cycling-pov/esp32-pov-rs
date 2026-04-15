#[cfg(feature = "sk9822-strip")]
mod sk9822_strip;
mod strip;
#[cfg(feature = "waveshare-matrix")]
mod waveshare_matrix;
use embassy_executor::Spawner;
#[cfg(feature = "waveshare-matrix")]
use esp_hal::rmt::Rmt;

#[cfg(feature = "waveshare-matrix")]
use esp_hal::time::Rate;
#[cfg(feature = "sk9822-strip")]
pub use sk9822_strip::{Sk9822Pins, Sk9822Strip};
pub use strip::{LedError, LedStrip, LedTimings};
#[cfg(feature = "waveshare-matrix")]
pub use waveshare_matrix::WaveshareMatrix;
#[cfg(feature = "waveshare-matrix")]
pub use waveshare_matrix::WaveshareMatrixPins;
#[cfg(feature = "waveshare-matrix")]
pub use waveshare_matrix::{WaveshareCommand, try_send_led_command};

pub async fn init(
    _rmt: esp_hal::peripherals::RMT<'static>,
    _waveshare_pin: esp_hal::peripherals::GPIO14<'static>,
    spawner: Spawner,
) {
    #[cfg(feature = "waveshare-matrix")]
    {
        let rmt = Rmt::new(_rmt, Rate::from_mhz(80)).expect("failed to initialize RMT");
        let led_strip =
            WaveshareMatrix::new(rmt.channel0, WaveshareMatrixPins::new(_waveshare_pin));
        spawner
            .spawn(waveshare_matrix::waveshare_matrix_task(led_strip))
            .expect("failed to spawn waveshare matrix task");
    }
}
