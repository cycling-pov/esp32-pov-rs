use core::time::Duration;

use esp_hal_smartled::LedAdapterError;
use smart_leds_trait::RGB8;

// WS2811: 24 bits per LED × 2,500 ns per bit = 60,000 ns per LED
const WS2811_DATA_TIME_PER_LED: Duration = Duration::from_nanos(24 * 2_500);
const WS2811_RESET_LATCH: Duration = Duration::from_micros(50);

#[derive(Clone, Copy, Debug, Eq, PartialEq, defmt::Format)]
pub struct LedTimings {
    /// Time to transmit one LED's worth of color data.
    pub data_time_per_led: Duration,
    /// Minimum idle time required between consecutive strip updates.
    pub reset_latch: Duration,
}

impl LedTimings {
    pub const WS2811: Self = Self {
        data_time_per_led: WS2811_DATA_TIME_PER_LED,
        reset_latch: WS2811_RESET_LATCH,
    };
}

#[derive(Debug, defmt::Format)]
pub enum LedError {
    InvalidIndex { index: usize, led_count: usize },
    Write(LedAdapterError),
}

impl From<LedAdapterError> for LedError {
    fn from(value: LedAdapterError) -> Self {
        Self::Write(value)
    }
}

pub trait LedStrip {
    fn led_count(&self) -> usize;

    fn timings(&self) -> LedTimings;

    fn pixels(&self) -> &[RGB8];

    fn pixels_mut(&mut self) -> &mut [RGB8];

    fn show(&mut self) -> Result<(), LedError>;

    /// Total time for one full strip update: data for all LEDs plus reset latch.
    fn refresh_period(&self) -> Duration {
        let timings = self.timings();
        let data_total = timings
            .data_time_per_led
            .saturating_mul(self.led_count() as u32);
        data_total.saturating_add(timings.reset_latch)
    }

    fn set_led(&mut self, index: usize, color: RGB8) -> Result<(), LedError> {
        let led_count = self.led_count();
        let Some(pixel) = self.pixels_mut().get_mut(index) else {
            return Err(LedError::InvalidIndex { index, led_count });
        };

        *pixel = color;
        Ok(())
    }

    fn fill(&mut self, color: RGB8) {
        for pixel in self.pixels_mut() {
            *pixel = color;
        }
    }

    fn clear(&mut self) {
        self.fill(RGB8::default());
    }
}
