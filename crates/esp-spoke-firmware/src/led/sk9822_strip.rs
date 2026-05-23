use defmt::{info, warn};
use embassy_futures::select::{Either, select};
use embassy_time::{Duration as EmbassyDuration, Timer};
use esp_hal::{
    Async,
    dma::DmaLoopBuf,
    gpio::{AnyPin, Pin},
    rng::Rng,
    spi::master::SpiDma,
};
use smart_leds_trait::RGB8;
use static_cell::StaticCell;

use crate::bitmap::{Bitmap, BitmapStorage, generated_swapping_storage};
use crate::led::task_common::{self, RenderBitmap};
use crate::led::{LedBrightness, LedCommand, LedError, LedStrip, LedTimings};

pub const SK9822_LED_COUNT: usize = 30;

// Must be large enough to hold the largest decompressed pixel payload:
// polar 30×360×3 = 32 400 bytes; Cartesian 64×64×3 = 12 288 bytes.
const SK9822_DECODE_SCRATCH_BYTES: usize = 1024 * 34;

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

impl<const LED_COUNT: usize> RenderBitmap for Sk9822Strip<'_, LED_COUNT> {
    async fn render_from_bitmap(&mut self, _bitmap: &Bitmap<'_>) {
        // Single-strip POV rendering is not implemented.
        // Use PovDualStrip for full POV display with spin-state-based sampling.
        info!("sk9822: single-strip render_from_bitmap is a no-op; use PovDualStrip");
    }
}

#[embassy_executor::task]
pub async fn sk9822_strip_task(mut led_strip: Sk9822Strip<'static, SK9822_LED_COUNT>) -> ! {
    info!(
        "SK9822 strip ready: leds={}, timings={:?}",
        led_strip.led_count(),
        led_strip.timings()
    );

    static DECODE_SCRATCH: StaticCell<[u8; SK9822_DECODE_SCRATCH_BYTES]> = StaticCell::new();
    let decode_scratch = DECODE_SCRATCH.init([0; SK9822_DECODE_SCRATCH_BYTES]);

    let mut bitmap_store = generated_swapping_storage();
    let mut randomizing = false;
    let rng = Rng::new();

    let mut current_display_slot =
        task_common::boot_restore(&mut *bitmap_store, decode_scratch).await;
    if current_display_slot.is_some() {
        info!("sk9822:boot active image is downloaded from flash");
        if let Ok(bitmap) = bitmap_store.bitmap(0) {
            led_strip.render_from_bitmap(&bitmap).await;
        }
    } else {
        info!("sk9822:boot no valid flash image; starting with built-in");
    }
    info!("rendered bitmap at startup");

    loop {
        let led_cmd = if randomizing {
            let delay = EmbassyDuration::from_millis(10);
            match select(super::LED_COMMAND_CHANNEL.receive(), Timer::after(delay)).await {
                Either::First(cmd) => Some(cmd),
                Either::Second(_) => {
                    led_strip.randomize(&rng);
                    led_strip
                        .show()
                        .await
                        .expect("failed to show randomized SK9822 strip");
                    None
                }
            }
        } else {
            Some(super::LED_COMMAND_CHANNEL.receive().await)
        };

        let Some(led_cmd) = led_cmd else { continue };
        randomizing = false;

        match led_cmd {
            LedCommand::Frame(frame) => {
                info!(
                    "sk9822:loop handling frame transfer_id={} command={:?}",
                    frame.transfer_id, frame.command
                );
                task_common::apply_led_command(
                    &mut led_strip,
                    &mut *bitmap_store,
                    &mut current_display_slot,
                    decode_scratch,
                    &mut randomizing,
                    frame,
                )
                .await;
            }
            LedCommand::LoadSlot(slot) => {
                info!("sk9822:loop load_slot slot={}", slot);
                if task_common::load_flash_slot(slot, &mut *bitmap_store, decode_scratch).await {
                    current_display_slot = Some(slot);
                    if let Ok(bitmap) = bitmap_store.bitmap(0) {
                        led_strip.render_from_bitmap(&bitmap).await;
                    }
                    info!("sk9822:loop loaded flash slot {}", slot);
                } else {
                    warn!("sk9822:loop failed to load flash slot {}", slot);
                }
            }
        }
    }
}
