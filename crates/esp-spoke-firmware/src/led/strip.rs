use core::time::Duration;

use esp_hal::rng::Rng;
use esp_hal_smartled::AdapterError as LedAdapterError;
use smart_leds_trait::RGB8;

// WS2811: 24 bits per LED × 2,500 ns per bit = 60,000 ns per LED
const WS2811_DATA_TIME_PER_LED: Duration = Duration::from_nanos(24 * 2_500);
const WS2811_RESET_LATCH: Duration = Duration::from_micros(50);

// SK9822: 32 bits per LED at 30 MHz = 1 us per LED.
const SK9822_DATA_TIME_PER_LED: Duration = Duration::from_micros(1);
// SK9822 Consider the start frame and end frame as the latch time. At 30 MHz, the overhead is 64 bits and thus 2 us.
const SK9822_RESET_LATCH: Duration = Duration::from_micros(2);

/// A 5-bit LED global brightness level for SK9822-compatible strips (0–31).
#[derive(Clone, Copy, Debug, Eq, PartialEq, defmt::Format)]
pub struct LedBrightness(u8);

impl LedBrightness {
    /// Constructs a brightness level. Values above 31 are treated as 31 at the hardware boundary.
    pub const fn new(v: u8) -> Self {
        Self(v)
    }

    /// Returns the raw brightness byte value.
    pub const fn value(self) -> u8 {
        self.0
    }
}

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

    pub const SK9822: Self = Self {
        data_time_per_led: SK9822_DATA_TIME_PER_LED,
        reset_latch: SK9822_RESET_LATCH,
    };
}

#[derive(Debug, defmt::Format)]
pub enum LedError {
    InvalidIndex { index: usize, led_count: usize },
    Write(LedAdapterError),
    SpiWrite,
}

impl From<LedAdapterError> for LedError {
    fn from(value: LedAdapterError) -> Self {
        Self::Write(value)
    }
}

#[allow(async_fn_in_trait, reason = "LedStrip is an internal firmware trait")]
pub trait LedStrip {
    fn led_count(&self) -> usize;

    /// Optional global brightness level for strips that support it.
    fn brightness(&self) -> Option<LedBrightness> {
        None
    }

    fn timings(&self) -> LedTimings;

    fn pixels(&self) -> &[RGB8];

    fn pixels_mut(&mut self) -> &mut [RGB8];

    async fn show(&mut self) -> Result<(), LedError>;

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

    fn randomize(&mut self, rng: &Rng) {
        for pixel in self.pixels_mut() {
            let value = rng.random();
            *pixel = RGB8 {
                r: (value & 0xFF) as u8,
                g: ((value >> 8) & 0xFF) as u8,
                b: ((value >> 16) & 0xFF) as u8,
            };
        }
    }
}
