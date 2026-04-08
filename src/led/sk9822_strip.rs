use alloc::vec;
use alloc::vec::Vec;

use esp_hal::{
    Blocking,
    gpio::{AnyPin, Pin},
    spi::master::Spi,
};
use smart_leds_trait::RGB8;

use crate::led::{LedError, LedStrip, LedTimings};

const SK9822_MAX_BRIGHTNESS: u8 = 31;
const SK9822_BRIGHTNESS_LIMIT_PERCENT: u8 = 5;
// SK9822 global brightness has 5 bits (0..31). 1/31 ~= 3.2%, 2/31 ~= 6.5%, so
// level 1 is the highest level that does not exceed 5%.
const SK9822_BRIGHTNESS: u8 =
    ((SK9822_MAX_BRIGHTNESS as u16 * SK9822_BRIGHTNESS_LIMIT_PERCENT as u16) / 100) as u8;
const SK9822_START_FRAME_BYTES: usize = 4;

const fn sk9822_end_frame_bytes(led_count: usize) -> usize {
    led_count.div_ceil(16)
}

const fn sk9822_frame_size(led_count: usize) -> usize {
    SK9822_START_FRAME_BYTES + (4 * led_count) + sk9822_end_frame_bytes(led_count)
}

pub struct Sk9822Pins<'d> {
    pub clock: AnyPin<'d>,
    pub data: AnyPin<'d>,
}

impl<'d> Sk9822Pins<'d> {
    pub fn new(clock: impl Pin + 'd, data: impl Pin + 'd) -> Self {
        Self {
            clock: clock.degrade(),
            data: data.degrade(),
        }
    }
}

pub struct Sk9822Strip<'d, const LED_COUNT: usize> {
    spi: Spi<'d, Blocking>,
    framebuffer: [RGB8; LED_COUNT],
    tx_buffer: Vec<u8>,
}

impl<'d, const LED_COUNT: usize> Sk9822Strip<'d, LED_COUNT> {
    pub const LED_COUNT: usize = LED_COUNT;
    pub const TIMINGS: LedTimings = LedTimings::SK9822;

    pub fn new(spi: Spi<'d, Blocking>, pins: Sk9822Pins<'d>) -> Self {
        let spi = spi.with_sck(pins.clock).with_mosi(pins.data);

        Self {
            spi,
            framebuffer: [RGB8::default(); LED_COUNT],
            tx_buffer: vec![0; sk9822_frame_size(LED_COUNT)],
        }
    }

    fn encode_framebuffer(&mut self) {
        self.tx_buffer.fill(0);

        for (index, pixel) in self.framebuffer.iter().copied().enumerate() {
            let offset = SK9822_START_FRAME_BYTES + (index * 4);
            self.tx_buffer[offset] = 0b1110_0000 | SK9822_BRIGHTNESS;
            self.tx_buffer[offset + 1] = pixel.b;
            self.tx_buffer[offset + 2] = pixel.g;
            self.tx_buffer[offset + 3] = pixel.r;
        }

        let end_start = SK9822_START_FRAME_BYTES + (LED_COUNT * 4);
        let end_count = sk9822_end_frame_bytes(LED_COUNT);
        for byte in &mut self.tx_buffer[end_start..end_start + end_count] {
            *byte = 0xFF;
        }
    }
}

impl<const LED_COUNT: usize> LedStrip for Sk9822Strip<'_, LED_COUNT> {
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
        self.encode_framebuffer();
        self.spi
            .write(&self.tx_buffer)
            .map_err(|_| LedError::SpiWrite)
    }
}
