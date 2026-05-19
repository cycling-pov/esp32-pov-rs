use core::f32::consts::PI;

use defmt::{info, warn};
use esp_hal::rng::Rng;
use smart_leds_trait::RGB8;
use static_cell::StaticCell;

use crate::angles::spin_estimator::SharedSpinState;
use crate::bitmap::{Bitmap, BitmapStorage, generated_swapping_storage};
use crate::led::sk9822_strip::{SK9822_LED_COUNT, Sk9822Strip};
use crate::led::task_common::{self, RenderBitmap};
use crate::led::{LedCommand, LedError, LedStrip, LedTimings};

/// Scratch buffer size: large enough for a full polar image (30×360×3 bytes).
pub const POV_DECODE_SCRATCH_BYTES: usize = 1024 * 34;

/// Two opposing SK9822 LED strips driven as a POV display.
///
/// `strip0` sits at the current arm angle θ; `strip1` sits at θ + π (the
/// opposite arm).  On each render call the spin estimator is consulted for the
/// current angular position and both strips are updated with the matching
/// radial slice of the active polar bitmap.
pub struct PovDualStrip<'d, const LEDS: usize> {
    strip0: Sk9822Strip<'d, LEDS>,
    strip1: Sk9822Strip<'d, LEDS>,
    spin: &'static SharedSpinState,
}

impl<'d, const LEDS: usize> PovDualStrip<'d, LEDS> {
    pub fn new(
        strip0: Sk9822Strip<'d, LEDS>,
        strip1: Sk9822Strip<'d, LEDS>,
        spin: &'static SharedSpinState,
    ) -> Self {
        Self {
            strip0,
            strip1,
            spin,
        }
    }
}

impl<const LEDS: usize> LedStrip for PovDualStrip<'_, LEDS> {
    fn led_count(&self) -> usize {
        LEDS
    }

    fn timings(&self) -> LedTimings {
        LedTimings::SK9822
    }

    /// Returns the framebuffer of `strip0`.  Use `fill` / `randomize` to
    /// write to both strips at once.
    fn pixels(&self) -> &[RGB8] {
        self.strip0.pixels()
    }

    fn pixels_mut(&mut self) -> &mut [RGB8] {
        self.strip0.pixels_mut()
    }

    async fn show(&mut self) -> Result<(), LedError> {
        self.strip0.show().await?;
        self.strip1.show().await
    }

    /// Fill both strips with the same colour.
    fn fill(&mut self, color: RGB8) {
        self.strip0.fill(color);
        self.strip1.fill(color);
    }

    /// Randomize both strips independently.
    fn randomize(&mut self, rng: &Rng) {
        self.strip0.randomize(rng);
        self.strip1.randomize(rng);
    }
}

impl<const LEDS: usize> RenderBitmap for PovDualStrip<'_, LEDS> {
    async fn render_from_bitmap(&mut self, bitmap: &Bitmap<'_>) {
        let num_radials = bitmap.height();
        let bm_width = bitmap.width();
        let pixels = bitmap.pixels();

        // Read the current angular position from the shared spin state.
        let angle_rad = self.spin.lock(|s| s.borrow().position.radians());

        let radial0 = radial_index(angle_rad, num_radials);
        let radial1 = radial_index(angle_rad + PI, num_radials);

        copy_radial(pixels, bm_width, radial0, self.strip0.pixels_mut());
        copy_radial(pixels, bm_width, radial1, self.strip1.pixels_mut());

        self.strip0.show().await.expect("pov: strip0 show failed");
        self.strip1.show().await.expect("pov: strip1 show failed");
    }
}

/// Normalises `angle_rad` to `[0, 2π)` and returns the corresponding row
/// index in `[0, num_radials)`.
fn radial_index(angle_rad: f32, num_radials: usize) -> usize {
    if num_radials == 0 {
        return 0;
    }
    let circle = 2.0 * PI;
    let r = angle_rad % circle;
    let r = if r < 0.0 { r + circle } else { r };
    let idx = (r / circle * num_radials as f32) as usize;
    idx % num_radials
}

/// Copies pixels from row `radial` of the flat `pixels` slice (row-major,
/// `bm_width` pixels wide) into `dest`.
///
/// If `bm_width > dest.len()`, only the first `dest.len()` source pixels are
/// copied.  If `bm_width < dest.len()`, the remaining destination pixels are
/// zeroed.  Out-of-range radial indices zero the destination.
fn copy_radial(pixels: &[RGB8], bm_width: usize, radial: usize, dest: &mut [RGB8]) {
    let row_start = radial * bm_width;
    let row_end = row_start + bm_width;

    if row_end > pixels.len() {
        for p in dest.iter_mut() {
            *p = RGB8::default();
        }
        return;
    }

    let src = &pixels[row_start..row_end];
    let copy_len = src.len().min(dest.len());
    dest[..copy_len].copy_from_slice(&src[..copy_len]);
    for p in &mut dest[copy_len..] {
        *p = RGB8::default();
    }
}

#[embassy_executor::task]
pub async fn pov_dual_strip_task(mut strips: PovDualStrip<'static, SK9822_LED_COUNT>) -> ! {
    info!(
        "POV dual-strip ready: leds={}, timings={:?}",
        strips.led_count(),
        strips.timings()
    );

    static DECODE_SCRATCH: StaticCell<[u8; POV_DECODE_SCRATCH_BYTES]> = StaticCell::new();
    let decode_scratch = DECODE_SCRATCH.init([0; POV_DECODE_SCRATCH_BYTES]);

    let mut bitmap_store = generated_swapping_storage();
    let rng = Rng::new();
    let mut randomizing = false;

    let mut current_display_slot =
        task_common::boot_restore(&mut *bitmap_store, decode_scratch).await;
    if current_display_slot.is_some() {
        info!("pov:boot active image is downloaded from flash");
    } else {
        info!("pov:boot no valid flash image; starting with built-in");
    }

    loop {
        // When an image is loaded or we are randomizing, render a frame then
        // check for commands without blocking.  Otherwise block on the next
        // command so we don't busy-loop.
        let led_cmd: Option<LedCommand> = if current_display_slot.is_some() || randomizing {
            if randomizing {
                strips.randomize(&rng);
                strips.show().await.expect("pov: randomize show failed");
            } else if let Ok(bitmap) = bitmap_store.bitmap(0) {
                strips.render_from_bitmap(&bitmap).await;
            }
            super::LED_COMMAND_CHANNEL.try_receive().ok()
        } else {
            Some(super::LED_COMMAND_CHANNEL.receive().await)
        };

        let Some(cmd) = led_cmd else {
            continue;
        };
        randomizing = false;

        match cmd {
            LedCommand::Frame(frame) => {
                info!(
                    "pov:loop handling frame transfer_id={} command={:?}",
                    frame.transfer_id, frame.command
                );
                task_common::apply_led_command(
                    &mut strips,
                    &mut *bitmap_store,
                    &mut current_display_slot,
                    decode_scratch,
                    &mut randomizing,
                    frame,
                )
                .await;
            }
            LedCommand::LoadSlot(slot) => {
                info!("pov:loop load_slot slot={}", slot);
                if task_common::load_flash_slot(slot, &mut *bitmap_store, decode_scratch).await {
                    current_display_slot = Some(slot);
                    info!("pov:loop loaded flash slot {}", slot);
                } else {
                    warn!("pov:loop failed to load flash slot {}", slot);
                }
            }
        }
    }
}
