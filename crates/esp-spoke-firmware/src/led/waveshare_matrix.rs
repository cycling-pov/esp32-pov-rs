use esp_hal::{
    Blocking,
    peripherals::GPIO14,
    rmt::{PulseCode, TxChannelCreator},
};
use esp_hal_smartled::{SmartLedsAdapter, buffer_size};
use smart_leds_trait::{RGB8, SmartLedsWrite as _};
use static_cell::StaticCell;

use crate::led::{LedError, LedStrip, LedTimings};

// The Waveshare Matrix has very poor thermal design. The manufacturer recommends limiting
// the brightness to 50%. We'll cap the brightness to 1% to prevent overheating and because
// the LEDs are very bright even at low brightness levels.
const WAVESHARE_MATRIX_BRIGHTNESS_LIMIT_PERCENT: u16 = 1;

const WAVESHARE_MATRIX_LED_COUNT: usize = 64;
const WAVESHARE_MATRIX_BUFFER_SIZE: usize = buffer_size(WAVESHARE_MATRIX_LED_COUNT);

fn apply_brightness_limit(color: RGB8) -> RGB8 {
    RGB8 {
        r: scale_channel(color.r),
        g: scale_channel(color.g),
        b: scale_channel(color.b),
    }
}

fn scale_channel(value: u8) -> u8 {
    ((value as u16 * WAVESHARE_MATRIX_BRIGHTNESS_LIMIT_PERCENT) / 100) as u8
}

pub struct WaveshareMatrixPins<'d> {
    data: GPIO14<'d>,
}

impl<'d> WaveshareMatrixPins<'d> {
    pub fn new(data: GPIO14<'d>) -> Self {
        Self { data }
    }
}

pub struct WaveshareMatrix<'d> {
    driver: SmartLedsAdapter<'d, WAVESHARE_MATRIX_BUFFER_SIZE, RGB8>,
    framebuffer: [RGB8; WAVESHARE_MATRIX_LED_COUNT],
}

impl<'d> WaveshareMatrix<'d> {
    pub const LED_COUNT: usize = WAVESHARE_MATRIX_LED_COUNT;
    pub const TIMINGS: LedTimings = LedTimings::WS2811;

    pub fn new<C>(channel: C, pins: WaveshareMatrixPins<'d>) -> Self
    where
        C: TxChannelCreator<'d, Blocking>,
    {
        static RMT_BUFFER: StaticCell<[PulseCode; WAVESHARE_MATRIX_BUFFER_SIZE]> =
            StaticCell::new();

        let rmt_buffer = RMT_BUFFER.init([PulseCode::end_marker(); WAVESHARE_MATRIX_BUFFER_SIZE]);

        Self {
            // Waveshare matrix LEDs use RGB byte order, not the more common GRB.
            driver: SmartLedsAdapter::new_with_color(channel, pins.data, rmt_buffer),
            framebuffer: [RGB8::default(); WAVESHARE_MATRIX_LED_COUNT],
        }
    }
}

impl LedStrip for WaveshareMatrix<'_> {
    fn led_count(&self) -> usize {
        self.framebuffer.len()
    }

    fn timings(&self) -> LedTimings {
        Self::TIMINGS
    }

    fn pixels(&self) -> &[RGB8] {
        &self.framebuffer
    }

    fn pixels_mut(&mut self) -> &mut [RGB8] {
        &mut self.framebuffer
    }

    fn show(&mut self) -> Result<(), LedError> {
        self.driver
            .write(self.framebuffer.iter().copied().map(apply_brightness_limit))
            .map_err(LedError::from)
    }
}
