use defmt::{info, warn};
use embassy_futures::select::{Either, select};
use embassy_time::{Duration as EmbassyDuration, Timer};
use esp_hal::{Blocking, peripherals::GPIO14, rmt::TxChannelCreator, rng::Rng};
use esp_hal_smartled::{RmtSmartLeds, Ws2811Timing, buffer_size, color_order};
use pov_proto::transfer::SpokeCommand;
use smart_leds_trait::{RGB8, SmartLedsWrite as _};
use static_cell::StaticCell;

use crate::bitmap::{Bitmap, BitmapStorage, generated_swapping_storage};
use crate::led::task_common;
use crate::led::{LedBrightness, LedCommand, LedError, LedStrip, LedTimings};

// The Waveshare Matrix has very poor thermal design. The manufacturer recommends limiting
// the brightness to 50%. We'll cap the brightness to 1% to prevent overheating and because
// the LEDs are very bright even at low brightness levels.
const WAVESHARE_MATRIX_BRIGHTNESS_LIMIT_PERCENT: u16 = 1;

const WAVESHARE_MATRIX_LED_COUNT: usize = 64;
const WAVESHARE_MATRIX_BUFFER_SIZE: usize = buffer_size::<RGB8>(WAVESHARE_MATRIX_LED_COUNT);
const WAVESHARE_MATRIX_BRIGHTNESS: LedBrightness = LedBrightness::new(1);

const WAVESHARE_DECODE_SCRATCH_BYTES: usize = 1024 * 10;

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
    driver: RmtSmartLeds<
        'd,
        WAVESHARE_MATRIX_BUFFER_SIZE,
        Blocking,
        RGB8,
        color_order::Rgb,
        Ws2811Timing,
    >,
    framebuffer: [RGB8; WAVESHARE_MATRIX_LED_COUNT],
}

impl<'d> WaveshareMatrix<'d> {
    pub const LED_COUNT: usize = WAVESHARE_MATRIX_LED_COUNT;
    pub const TIMINGS: LedTimings = LedTimings::WS2811;

    pub fn new<C>(channel: C, pins: WaveshareMatrixPins<'d>) -> Self
    where
        C: TxChannelCreator<'d, Blocking>,
    {
        Self {
            // Waveshare matrix LEDs use RGB byte order, not the more common GRB.
            driver: RmtSmartLeds::new(channel, pins.data).expect("failed to configure LED RMT"),
            framebuffer: [RGB8::default(); WAVESHARE_MATRIX_LED_COUNT],
        }
    }

    async fn render_from_bitmap(&mut self, bitmap: &Bitmap<'_>) {
        let target_width = 8;
        let target_height = WaveshareMatrix::LED_COUNT / target_width;
        bitmap
            .scale_into(target_width, target_height, self.pixels_mut())
            .expect("failed to scale bitmap into Waveshare matrix");
        self.show().await.expect("failed to update LED strip");
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

#[embassy_executor::task]
pub async fn waveshare_matrix_task(mut led_strip: WaveshareMatrix<'static>) -> ! {
    info!(
        "Waveshare matrix ready: leds={}, timings={:?}",
        led_strip.led_count(),
        led_strip.timings()
    );

    static DECODE_SCRATCH: StaticCell<[u8; WAVESHARE_DECODE_SCRATCH_BYTES]> = StaticCell::new();
    let decode_scratch = DECODE_SCRATCH.init([0; WAVESHARE_DECODE_SCRATCH_BYTES]);

    let mut bitmap_store = generated_swapping_storage();
    let mut randomizing = false;
    let rng = Rng::new();

    let mut current_display_slot =
        task_common::boot_restore(&mut *bitmap_store, decode_scratch).await;
    if current_display_slot.is_some() {
        info!("waveshare:boot active image is downloaded from flash");
        if let Ok(bitmap) = bitmap_store.bitmap(0) {
            led_strip.render_from_bitmap(&bitmap).await;
        }
    } else {
        info!("waveshare:boot no valid flash image; starting with built-in");
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

                match frame.command {
                    SpokeCommand::DisplayOff => {
                        led_strip.clear();
                        led_strip.show().await.expect("failed to clear LED strip");
                        info!("applied DisplayOff from transfer {}", frame.transfer_id);
                    }
                    SpokeCommand::NextImage => {
                        let next_slot = match current_display_slot {
                            None => Some(0usize),
                            Some(0) => Some(1),
                            Some(_) => None,
                        };
                        current_display_slot = next_slot;
                        match next_slot {
                            None => {
                                bitmap_store.activate_builtin();
                                if let Ok(bitmap) = bitmap_store.bitmap(0) {
                                    led_strip.render_from_bitmap(&bitmap).await;
                                }
                            }
                            Some(slot) => {
                                if task_common::load_flash_slot(
                                    slot,
                                    &mut *bitmap_store,
                                    decode_scratch,
                                )
                                .await
                                {
                                    if let Ok(bitmap) = bitmap_store.bitmap(0) {
                                        led_strip.render_from_bitmap(&bitmap).await;
                                    }
                                } else {
                                    led_strip.clear();
                                    led_strip.show().await.expect("failed to clear LED strip");
                                }
                            }
                        }
                        info!(
                            "applied NextImage from transfer {}: display_slot={:?}",
                            frame.transfer_id, current_display_slot
                        );
                    }
                    SpokeCommand::RandomizeDisplay => {
                        randomizing = true;
                        info!(
                            "applied RandomizeDisplay from transfer {}",
                            frame.transfer_id
                        );
                    }
                    SpokeCommand::SetSensorOffsets { .. } => {
                        info!(
                            "ignoring SetSensorOffsets in LED task for transfer {}",
                            frame.transfer_id
                        );
                    }
                }
            }
            LedCommand::LoadSlot(slot) => {
                info!("waveshare:loop load_slot slot={}", slot);
                if task_common::load_flash_slot(slot, &mut *bitmap_store, decode_scratch).await {
                    current_display_slot = Some(slot);
                    if let Ok(bitmap) = bitmap_store.bitmap(0) {
                        led_strip.render_from_bitmap(&bitmap).await;
                    }
                    info!("waveshare:loop loaded flash slot {}", slot);
                } else {
                    warn!("waveshare:loop failed to load flash slot {}", slot);
                }
            }
        }
    }
}
