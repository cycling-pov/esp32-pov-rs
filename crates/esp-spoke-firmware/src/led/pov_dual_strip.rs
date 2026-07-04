use alloc::boxed::Box;
use core::f32::consts::PI;
use core::sync::atomic::{AtomicBool, Ordering};

use defmt::{info, warn};
use embassy_futures::join::join;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};
use esp_hal::rng::Rng;
use pov_proto::transfer::SpokeCommand;
use smart_leds_trait::RGB8;
use static_cell::StaticCell;

use crate::angle_estimator::SharedSpinState;
use crate::bitmap::{
    Bitmap, BitmapStorage, MAX_POLAR_PIXEL_COUNT, SwappingImageStorage, generated_swapping_storage,
};
use crate::led::sk9822_strip::{SK9822_LED_COUNT, Sk9822Strip};
use crate::led::task_common;
use crate::led::{CORE1_FLASH_PAUSE_REQUESTED, CORE1_FLASH_PAUSED_COUNT};
use crate::led::{LedCommand, LedError, LedStrip, LedTimings};
#[cfg(feature = "status-led")]
use crate::status_led::{self, StatusLedRequest};

/// Scratch buffer size: large enough for a full polar image (30×360×3 bytes).
pub const POV_DECODE_SCRATCH_BYTES: usize = 1024 * 34;

// ---------------------------------------------------------------------------
// Shared bitmap state
// ---------------------------------------------------------------------------

/// Heap-allocated swapping image storage shared between the render and command tasks.
type BitmapStore = Box<SwappingImageStorage<MAX_POLAR_PIXEL_COUNT>>;

/// Async mutex protecting the shared [`BitmapStore`].
///
/// The render task briefly locks it on each frame (just long enough to copy
/// one radial slice per strip), then releases the lock before calling `show()`.
/// The command task locks it for the duration of a flash-slot decode.  The
/// mutex is therefore never held across an SPI transfer.
pub type SharedBitmapMutex = Mutex<CriticalSectionRawMutex, BitmapStore>;

static SHARED_BITMAP: StaticCell<SharedBitmapMutex> = StaticCell::new();

/// `true` while the render task should output polar bitmap frames.
/// Cleared on [`SpokeCommand::DisplayOff`] or when no image is available.
static RENDERING_ACTIVE: AtomicBool = AtomicBool::new(false);

/// `true` while the render task should output random-noise frames.
/// Cleared when any other display command is received.
static RANDOMIZING: AtomicBool = AtomicBool::new(false);

/// Busy-spins in IRAM while flash is being written.
///
/// Placed in IRAM via `#[esp_hal::ram]` so the CPU never fetches from
/// flash-backed ICache pages during the spin.  `Cache_Disable_ICache()` (called
/// inside ROM flash-write routines) is therefore harmless to this core.
#[esp_hal::ram]
fn render_pause_spin() {
    CORE1_FLASH_PAUSED_COUNT.fetch_add(1, Ordering::Release);
    while CORE1_FLASH_PAUSE_REQUESTED.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }
    CORE1_FLASH_PAUSED_COUNT.fetch_sub(1, Ordering::Release);
}

/// Initialise the shared bitmap store and return a `'static` reference to it.
///
/// Must be called exactly once (from [`crate::led::init_sk9822_dual`]) before
/// spawning [`pov_render_task`] or [`pov_command_task`].
pub fn init_bitmap_store() -> &'static SharedBitmapMutex {
    SHARED_BITMAP.init(Mutex::new(generated_swapping_storage()))
}

/// Two SK9822 LED strips driven as a POV display.
///
/// Each strip tracks its own angular position via an independent hall-effect
/// sensor.  On each render call both spin estimators are consulted separately
/// so that strip0 and strip1 each render the radial slice that corresponds to
/// their own measured angle — no fixed geometric offset is assumed.
pub struct PovDualStrip<'d, const LEDS: usize> {
    strip0: Sk9822Strip<'d, LEDS>,
    strip1: Sk9822Strip<'d, LEDS>,
    spin0: &'static SharedSpinState,
    spin1: &'static SharedSpinState,
}

// SAFETY: `PovDualStrip` is exclusively owned by a single task/executor.
// See `Sk9822Strip`'s Send impl for the rationale.
unsafe impl<'d, const N: usize> Send for PovDualStrip<'d, N> {}

impl<'d, const LEDS: usize> PovDualStrip<'d, LEDS> {
    pub fn new(
        strip0: Sk9822Strip<'d, LEDS>,
        strip1: Sk9822Strip<'d, LEDS>,
        spin0: &'static SharedSpinState,
        spin1: &'static SharedSpinState,
    ) -> Self {
        Self {
            strip0,
            strip1,
            spin0,
            spin1,
        }
    }
}

impl<const LEDS: usize> PovDualStrip<'_, LEDS> {
    /// Samples both spin estimators and copies the matching polar radial from
    /// `bitmap` into each strip's framebuffer.  Does **not** call `show()`.
    fn sample_and_copy_radials(&mut self, bitmap: &Bitmap<'_>) {
        let angle0_rad = self.spin0.lock(|s| s.borrow().position.radians());
        let angle1_rad = self.spin1.lock(|s| s.borrow().position.radians());
        let num_radials = bitmap.height();
        let bm_width = bitmap.width();
        let pixels = bitmap.pixels();
        let radial0 = radial_index(angle0_rad, num_radials);
        let radial1 = radial_index(angle1_rad, num_radials);
        copy_radial(pixels, bm_width, radial0, self.strip0.pixels_mut());
        copy_radial(pixels, bm_width, radial1, self.strip1.pixels_mut());
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
        let (s0, s1) = (&mut self.strip0, &mut self.strip1);
        let (r0, r1) = join(s0.show(), s1.show()).await;
        r0?;
        r1
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

/// Background render task: continuously samples both spin estimators and drives
/// both SPI outputs in parallel.
///
/// When [`RENDERING_ACTIVE`] is `true` the task locks the shared bitmap store
/// for a few hundred nanoseconds (long enough to copy one radial slice per
/// strip), releases the lock, then fires both SPI/DMA channels concurrently
/// via [`join`].  The mutex is **never** held across a `show()` call, so image
/// loading in [`pov_command_task`] is never blocked by an in-progress SPI
/// transfer.
#[embassy_executor::task]
pub async fn pov_render_task(
    mut strips: PovDualStrip<'static, SK9822_LED_COUNT>,
    bitmap: &'static SharedBitmapMutex,
) -> ! {
    info!(
        "POV render task started: leds={}, timings={:?}",
        strips.led_count(),
        strips.timings()
    );

    let rng = Rng::new();

    loop {
        if CORE1_FLASH_PAUSE_REQUESTED.load(Ordering::Acquire) {
            info!("pov:render paused for flash write");
            render_pause_spin();
            info!("pov:render resumed after flash write");
            continue;
        }

        if RANDOMIZING.load(Ordering::Relaxed) {
            strips.randomize(&rng);
            let (s0, s1) = (&mut strips.strip0, &mut strips.strip1);
            let (r0, r1) = join(s0.show(), s1.show()).await;
            r0.expect("pov: strip0 show failed (randomize)");
            r1.expect("pov: strip1 show failed (randomize)");
        } else if RENDERING_ACTIVE.load(Ordering::Relaxed) {
            {
                let guard = bitmap.lock().await;
                if let Ok(b) = guard.bitmap(0) {
                    strips.sample_and_copy_radials(&b);
                }
                // guard dropped here — mutex released before SPI transfers begin
            }
            let (s0, s1) = (&mut strips.strip0, &mut strips.strip1);
            let (r0, r1) = join(s0.show(), s1.show()).await;
            r0.expect("pov: strip0 show failed");
            r1.expect("pov: strip1 show failed");
        } else {
            strips.strip0.clear();
            strips.strip1.clear();
            strips
                .strip0
                .show()
                .await
                .expect("pov: strip0 clear failed");
            strips
                .strip1
                .show()
                .await
                .expect("pov: strip1 clear failed");
            // Nothing to display — yield briefly to avoid a busy-loop.
            Timer::after(Duration::from_millis(1)).await;
        }
    }
}

/// Command task: blocks on [`super::LED_COMMAND_CHANNEL`] and processes LED
/// control commands.  Runs concurrently with [`pov_render_task`] so the
/// render loop is never paused by command dispatch.
#[embassy_executor::task]
pub async fn pov_command_task(bitmap: &'static SharedBitmapMutex) -> ! {
    info!("POV command task started");

    static DECODE_SCRATCH: StaticCell<[u8; POV_DECODE_SCRATCH_BYTES]> = StaticCell::new();
    let decode_scratch = DECODE_SCRATCH.init([0; POV_DECODE_SCRATCH_BYTES]);

    // Attempt to restore the last-used flash image.  The bitmap mutex is held
    // for the duration; the render task is idle (RENDERING_ACTIVE = false) so
    // there is no contention.
    let initial_slot = {
        let mut guard = bitmap.lock().await;
        task_common::boot_restore(&mut **guard, decode_scratch).await
    };
    if initial_slot.is_some() {
        RENDERING_ACTIVE.store(true, Ordering::Relaxed);
        info!("pov:boot active image is downloaded from flash");
        #[cfg(feature = "status-led")]
        {
            let _ = status_led::try_send_request(StatusLedRequest::BLINK_SLOW);
        }
    } else {
        info!("pov:boot no valid flash image; starting with built-in");
    }

    // Tracks which flash slot is active so NextImage can cycle in order:
    // None → slot 0 → slot 1 → None → …
    let mut current_display_slot = initial_slot;

    loop {
        let cmd = super::LED_COMMAND_CHANNEL.receive().await;

        match cmd {
            LedCommand::Frame(frame) => {
                info!(
                    "pov:cmd transfer_id={} command={:?}",
                    frame.transfer_id, frame.command
                );

                match frame.command {
                    SpokeCommand::DisplayOff => {
                        RANDOMIZING.store(false, Ordering::Relaxed);
                        RENDERING_ACTIVE.store(false, Ordering::Relaxed);
                        #[cfg(feature = "status-led")]
                        {
                            let _ = status_led::try_send_request(StatusLedRequest::OFF);
                        }
                        info!("pov:cmd DisplayOff");
                    }

                    SpokeCommand::RandomizeDisplay => {
                        RANDOMIZING.store(true, Ordering::Relaxed);
                        #[cfg(feature = "status-led")]
                        {
                            let _ = status_led::try_send_request(StatusLedRequest::BLINK_FAST);
                        }
                        info!("pov:cmd RandomizeDisplay");
                    }

                    SpokeCommand::NextImage => {
                        RANDOMIZING.store(false, Ordering::Relaxed);
                        #[cfg(feature = "status-led")]
                        {
                            let _ = status_led::try_send_request(StatusLedRequest::BLINK_SLOW);
                        }
                        let next_slot = match current_display_slot {
                            None => Some(0usize),
                            Some(0) => Some(1),
                            Some(_) => None,
                        };
                        current_display_slot = next_slot;

                        let mut guard = bitmap.lock().await;
                        match next_slot {
                            None => {
                                guard.activate_builtin();
                                // Render the built-in image continuously.
                                RENDERING_ACTIVE.store(true, Ordering::Relaxed);
                            }
                            Some(slot) => {
                                if task_common::load_flash_slot(slot, &mut **guard, decode_scratch)
                                    .await
                                {
                                    RENDERING_ACTIVE.store(true, Ordering::Relaxed);
                                } else {
                                    RENDERING_ACTIVE.store(false, Ordering::Relaxed);
                                    warn!("pov:cmd NextImage failed to load slot {}", slot);
                                }
                            }
                        }
                        info!("pov:cmd NextImage display_slot={:?}", current_display_slot);
                    }

                    SpokeCommand::SetSensorOffsets { .. } => {
                        info!("pov:cmd ignoring SetSensorOffsets in render task");
                    }
                }
            }

            LedCommand::LoadSlot(slot) => {
                info!("pov:cmd load_slot slot={}", slot);
                RANDOMIZING.store(false, Ordering::Relaxed);
                let mut guard = bitmap.lock().await;
                if task_common::load_flash_slot(slot, &mut **guard, decode_scratch).await {
                    current_display_slot = Some(slot);
                    RENDERING_ACTIVE.store(true, Ordering::Relaxed);
                    #[cfg(feature = "status-led")]
                    {
                        let _ = status_led::try_send_request(StatusLedRequest::BLINK_SLOW);
                    }
                    info!("pov:cmd loaded flash slot {}", slot);
                } else {
                    warn!("pov:cmd failed to load flash slot {}", slot);
                    // Keep the current rendering state — old image stays on screen.
                }
            }
        }
    }
}
