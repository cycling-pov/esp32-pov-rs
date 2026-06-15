use crate::led::{LedBrightness, LedError, LedStrip, LedTimings};
use esp_hal::{
    Async,
    dma::DmaLoopBuf,
    gpio::{AnyPin, Pin},
    spi::master::SpiDma,
};
use smart_leds_trait::RGB8;

pub const SK9822_LED_COUNT: usize = 30;

const SK9822_MAX_BRIGHTNESS: LedBrightness = LedBrightness::new(31);
const SK9822_BRIGHTNESS_LIMIT_PERCENT: u8 = 5;
// SK9822 global brightness has 5 bits (0..31). 1/31 ~= 3.2%, 2/31 ~= 6.5%, so
// level 1 is the highest level that does not exceed 5%.
const SK9822_BRIGHTNESS: LedBrightness = LedBrightness::new(
    ((SK9822_MAX_BRIGHTNESS.value() as u16 * SK9822_BRIGHTNESS_LIMIT_PERCENT as u16) / 100) as u8,
);
const SK9822_START_FRAME_BYTES: usize = 4;

const fn sk9822_end_frame_bytes(led_count: usize) -> usize {
    led_count.div_ceil(16)
}

pub const fn sk9822_frame_size(led_count: usize) -> usize {
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
    spi: Option<SpiDma<'d, Async>>,
    dma_buf: Option<DmaLoopBuf>,
    framebuffer: [RGB8; LED_COUNT],
}

// SAFETY: `Sk9822Strip` is exclusively owned by a single task. After an
// ownership transfer across core boundaries, the originating core never
// accesses it again. The `!Send` derives from `SpiDma<Async>`'s
// `PhantomData<*const ()>`, which is a conservative lint rather than a
// true memory-safety hazard for a complete ownership handoff.
unsafe impl<'d, const N: usize> Send for Sk9822Strip<'d, N> {}

impl<'d, const LED_COUNT: usize> Sk9822Strip<'d, LED_COUNT> {
    pub const LED_COUNT: usize = LED_COUNT;
    pub const TIMINGS: LedTimings = LedTimings::SK9822;

    pub fn new(spi: SpiDma<'d, Async>, dma_buf: DmaLoopBuf) -> Self {
        Self {
            spi: Some(spi),
            dma_buf: Some(dma_buf),
            framebuffer: [RGB8::default(); LED_COUNT],
        }
    }

    fn encode_framebuffer(&self, buf: &mut [u8]) {
        buf[..SK9822_START_FRAME_BYTES].fill(0);

        for (index, pixel) in self.framebuffer.iter().copied().enumerate() {
            let offset = SK9822_START_FRAME_BYTES + (index * 4);
            buf[offset] = 0b1110_0000 | SK9822_BRIGHTNESS.value();
            buf[offset + 1] = pixel.b;
            buf[offset + 2] = pixel.g;
            buf[offset + 3] = pixel.r;
        }

        let end_start = SK9822_START_FRAME_BYTES + (LED_COUNT * 4);
        let end_count = sk9822_end_frame_bytes(LED_COUNT);
        for byte in &mut buf[end_start..end_start + end_count] {
            *byte = 0xFF;
        }
    }
}

impl<const LED_COUNT: usize> LedStrip for Sk9822Strip<'_, LED_COUNT> {
    fn led_count(&self) -> usize {
        self.framebuffer.len()
    }

    fn brightness(&self) -> Option<LedBrightness> {
        Some(SK9822_BRIGHTNESS)
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

    async fn show(&mut self) -> Result<(), LedError> {
        let mut dma_buf = self.dma_buf.take().expect("dma_buf missing");
        self.encode_framebuffer(&mut dma_buf);

        let frame_size = sk9822_frame_size(LED_COUNT);
        let spi = self.spi.take().expect("spi missing");
        let mut transfer = match spi.write(frame_size, dma_buf) {
            Ok(t) => t,
            Err((_, spi, dma_buf)) => {
                self.spi = Some(spi);
                self.dma_buf = Some(dma_buf);
                return Err(LedError::SpiWrite);
            }
        };

        transfer.wait_for_done().await;
        let (spi, dma_buf) = transfer.wait();
        self.spi = Some(spi);
        self.dma_buf = Some(dma_buf);
        Ok(())
    }
}
