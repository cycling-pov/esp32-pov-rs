use defmt::{info, warn};
use embassy_futures::select::{Either, select};
use embassy_time::{Duration as EmbassyDuration, Timer};
use esp_hal::{
    Blocking,
    peripherals::GPIO14,
    rmt::{PulseCode, TxChannelCreator},
    rng::Rng,
};
use esp_hal_smartled::{SmartLedsAdapter, buffer_size};
use pov_proto::image::{DecodeMode, decode_into_rgb8};
use pov_proto::transfer::{CommandFrame, SpokeCommand};
use smart_leds_trait::{RGB8, SmartLedsWrite as _};
use static_cell::StaticCell;

use crate::bitmap::{BitmapStorage, generated_swapping_storage};
use crate::led::{LedBrightness, LedCommand, LedError, LedStrip, LedTimings};
use crate::storage;
use crate::storage::config::ImageSlotState;

// The Waveshare Matrix has very poor thermal design. The manufacturer recommends limiting
// the brightness to 50%. We'll cap the brightness to 1% to prevent overheating and because
// the LEDs are very bright even at low brightness levels.
const WAVESHARE_MATRIX_BRIGHTNESS_LIMIT_PERCENT: u16 = 1;

const WAVESHARE_MATRIX_LED_COUNT: usize = 64;
const WAVESHARE_MATRIX_BUFFER_SIZE: usize = buffer_size(WAVESHARE_MATRIX_LED_COUNT);
const WAVESHARE_MATRIX_BRIGHTNESS: LedBrightness = LedBrightness::new(1);

const WAVESHARE_DECODE_SCRATCH_BYTES: usize = 1024 * 10;

async fn load_flash_slot(
    slot: usize,
    bitmap_store: &mut impl BitmapStorage,
    decode_scratch: &mut [u8],
) -> bool {
    let state = storage::get_slot_state(slot).await;
    if let ImageSlotState::Valid { .. } = state {
        match storage::read_slot_data(slot).await {
            Ok(img_bytes) => {
                if let Ok(mut writable) = bitmap_store.bitmap_mut(0) {
                    match decode_into_rgb8(
                        &img_bytes,
                        decode_scratch,
                        writable.pixels_mut(),
                        DecodeMode::ExactPixels,
                    ) {
                        Ok(_) => {
                            bitmap_store.activate_downloaded();
                            return true;
                        }
                        Err(err) => {
                            info!("waveshare:load flash slot {} decode error: {:?}", slot, err);
                        }
                    }
                }
            }
            Err(()) => info!("waveshare:load flash slot {} read error", slot),
        }
    }
    false
}

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

    fn brightness(&self) -> Option<LedBrightness> {
        Some(WAVESHARE_MATRIX_BRIGHTNESS)
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
        self.driver
            .write(self.framebuffer.iter().copied().map(apply_brightness_limit))
            .map_err(LedError::from)
    }
}

async fn render_active_bitmap(
    led_strip: &mut WaveshareMatrix<'_>,
    bitmap_store: &impl BitmapStorage,
) {
    let image_bitmap = bitmap_store.bitmap(0).expect("missing bitmap");
    let target_width = 8;
    let target_height = WaveshareMatrix::LED_COUNT / target_width;

    image_bitmap
        .scale_into(target_width, target_height, led_strip.pixels_mut())
        .expect("failed to scale bitmap");

    led_strip.show().await.expect("failed to update LED strip");
}

async fn apply_command(
    led_strip: &mut WaveshareMatrix<'_>,
    bitmap_store: &mut impl BitmapStorage,
    current_display_slot: &mut Option<usize>,
    decode_scratch: &mut [u8],
    randomizing: &mut bool,
    frame: CommandFrame,
) {
    info!(
        "waveshare:command received transfer_id={} command={:?}",
        frame.transfer_id, frame.command
    );

    match frame.command {
        SpokeCommand::DisplayOff => {
            *randomizing = false;
            led_strip.clear();
            led_strip.show().await.expect("failed to clear LED strip");
            info!(
                "applied DisplayOff command from transfer {}",
                frame.transfer_id
            );
        }
        SpokeCommand::NextImage => {
            *randomizing = false;
            // Cycle: None (built-in) → Some(0) → Some(1) → None → ...
            let next_slot = match *current_display_slot {
                None => Some(0usize),
                Some(0) => Some(1),
                Some(_) => None,
            };
            *current_display_slot = next_slot;
            match next_slot {
                None => {
                    bitmap_store.activate_builtin();
                    render_active_bitmap(led_strip, bitmap_store).await;
                }
                Some(slot) => {
                    if load_flash_slot(slot, bitmap_store, decode_scratch).await {
                        render_active_bitmap(led_strip, bitmap_store).await;
                    } else {
                        led_strip.clear();
                        led_strip.show().await.expect("failed to clear LED strip");
                    }
                }
            }
            info!(
                "applied NextImage command from transfer {}: display_slot={:?}",
                frame.transfer_id, *current_display_slot
            );
        }
        SpokeCommand::RandomizeDisplay => {
            *randomizing = true;
            info!(
                "applied RandomizeDisplay command from transfer {}",
                frame.transfer_id
            );
        }
    }
}

#[embassy_executor::task]
pub async fn waveshare_matrix_task(mut led_strip: WaveshareMatrix<'static>) -> ! {
    info!(
        "LED strip ready: leds={}, timings={:?}",
        led_strip.led_count(),
        led_strip.timings()
    );

    static DECODE_SCRATCH: StaticCell<[u8; WAVESHARE_DECODE_SCRATCH_BYTES]> = StaticCell::new();
    let decode_scratch = DECODE_SCRATCH.init([0; WAVESHARE_DECODE_SCRATCH_BYTES]);

    let mut bitmap_store = generated_swapping_storage();
    let mut current_display_slot: Option<usize> = None;
    let mut randomizing = false;
    let rng = Rng::new();

    // Boot restore: load the active flash slot into the download buffer.
    let active_flash_slot = storage::get_active_slot().await;
    // Try the active slot first, fall back to the other slot.
    let slots_to_try: [usize; 2] = match active_flash_slot {
        Some(s) => [s as usize, (s as usize + 1) % 2],
        None => [0, 1],
    };
    for &slot in &slots_to_try {
        let state = storage::get_slot_state(slot).await;
        info!("waveshare:boot slot={} state={:?}", slot, state);
        if let ImageSlotState::Valid { .. } = state {
            if load_flash_slot(slot, &mut *bitmap_store, decode_scratch).await {
                info!(
                    "waveshare:boot restored flash slot {} into download buffer",
                    slot
                );
                current_display_slot = Some(slot);
                break;
            } else {
                info!("waveshare:boot failed to load flash slot {}", slot);
            }
        }
    }
    if current_display_slot.is_some() {
        info!("waveshare:boot active image is downloaded from flash");
    } else {
        info!("waveshare:boot no valid flash image; starting with built-in");
    }

    render_active_bitmap(&mut led_strip, &*bitmap_store).await;
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
                        .expect("failed to show randomized Waveshare matrix");
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
                    "waveshare:loop handling frame transfer_id={} command={:?}",
                    frame.transfer_id, frame.command
                );
                apply_command(
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
                info!("waveshare:loop load_slot slot={}", slot);
                if load_flash_slot(slot, &mut *bitmap_store, decode_scratch).await {
                    current_display_slot = Some(slot);
                    render_active_bitmap(&mut led_strip, &*bitmap_store).await;
                    info!("waveshare:loop loaded flash slot {}", slot);
                } else {
                    warn!("waveshare:loop failed to load flash slot {}", slot);
                }
            }
        }
    }
}
