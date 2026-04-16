use defmt::info;
use embassy_futures::select::{Either, select};
use embassy_time::{Duration as EmbassyDuration, Timer};
use esp_hal::{
    Async,
    peripherals::GPIO14,
    rmt::{PulseCode, TxChannelCreator},
    rng::Rng,
};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async};
use pov_proto::image::{DecodeMode, decode_into_rgb8};
use pov_proto::transfer::{CommandFrame, DownloadKind, SpokeCommand};
use smart_leds_trait::{RGB8, SmartLedsWriteAsync as _};
use static_cell::StaticCell;

use crate::bitmap::{BitmapStorage, generated_image_storage};
use crate::led::{LedCommand, LedError, LedStrip, LedTimings};
use crate::networking::CompletedDownload;

// The Waveshare Matrix has very poor thermal design. The manufacturer recommends limiting
// the brightness to 50%. We'll cap the brightness to 1% to prevent overheating and because
// the LEDs are very bright even at low brightness levels.
const WAVESHARE_MATRIX_BRIGHTNESS_LIMIT_PERCENT: u16 = 1;

const WAVESHARE_MATRIX_LED_COUNT: usize = 64;
const WAVESHARE_MATRIX_BUFFER_SIZE: usize = buffer_size_async(WAVESHARE_MATRIX_LED_COUNT);

const WAVESHARE_RGB565_DECODE_SCRATCH_BYTES: usize = 1024 * 10;
const DOWNLOADABLE_IMAGE_SLOTS: usize = 2;

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
    driver: SmartLedsAdapterAsync<'d, WAVESHARE_MATRIX_BUFFER_SIZE, RGB8>,
    framebuffer: [RGB8; WAVESHARE_MATRIX_LED_COUNT],
}

impl<'d> WaveshareMatrix<'d> {
    pub const LED_COUNT: usize = WAVESHARE_MATRIX_LED_COUNT;
    pub const TIMINGS: LedTimings = LedTimings::WS2811;

    pub fn new<C>(channel: C, pins: WaveshareMatrixPins<'d>) -> Self
    where
        C: TxChannelCreator<'d, Async>,
    {
        static RMT_BUFFER: StaticCell<[PulseCode; WAVESHARE_MATRIX_BUFFER_SIZE]> =
            StaticCell::new();

        let rmt_buffer = RMT_BUFFER.init([PulseCode::end_marker(); WAVESHARE_MATRIX_BUFFER_SIZE]);

        Self {
            // Waveshare matrix LEDs use RGB byte order, not the more common GRB.
            driver: SmartLedsAdapterAsync::new_with_color(channel, pins.data, rmt_buffer),
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

    async fn show(&mut self) -> Result<(), LedError> {
        self.driver
            .write(self.framebuffer.iter().copied().map(apply_brightness_limit))
            .await
            .map_err(LedError::from)
    }
}

async fn render_bitmap_index(
    led_strip: &mut WaveshareMatrix<'_>,
    bitmap_store: &impl BitmapStorage,
    index: usize,
) {
    let image_bitmap = bitmap_store.bitmap(index).expect("missing bitmap");
    let target_width = 8;
    let target_height = WaveshareMatrix::LED_COUNT / target_width;

    image_bitmap
        .scale_into(target_width, target_height, led_strip.pixels_mut())
        .expect("failed to scale bitmap");

    led_strip.show().await.expect("failed to update LED strip");
}

async fn apply_downloaded_image(
    led_strip: &mut WaveshareMatrix<'_>,
    bitmap_store: &mut impl BitmapStorage,
    current_bitmap_index: &mut usize,
    next_download_slot: &mut usize,
    decode_scratch: &mut [u8],
    download: &CompletedDownload,
) {
    let metadata = bitmap_store.metadata();
    let pixel_count = metadata.pixel_count();

    let writable_base = bitmap_store
        .bitmap_count()
        .saturating_sub(DOWNLOADABLE_IMAGE_SLOTS);
    let writable_index = writable_base + (*next_download_slot % DOWNLOADABLE_IMAGE_SLOTS);
    *next_download_slot = (*next_download_slot + 1) % DOWNLOADABLE_IMAGE_SLOTS;
    let mut writable = bitmap_store
        .bitmap_mut(writable_index)
        .expect("missing writable image slot");

    let decoded = match decode_into_rgb8(
        download.payload(),
        decode_scratch,
        writable.pixels_mut(),
        DecodeMode::ExactPixels,
    ) {
        Ok(decoded) => decoded,
        Err(err) => {
            info!(
                "ignoring transfer {}: failed to decode framed payload ({:?})",
                download.transfer_id, err
            );
            return;
        }
    };
    info!(
        "decoded transfer {} as {:?} ({} bytes, {} pixels)",
        download.transfer_id, decoded, download.len, pixel_count
    );

    let target_width = 8;
    let target_height = WaveshareMatrix::LED_COUNT / target_width;
    writable
        .as_bitmap()
        .scale_into(target_width, target_height, led_strip.pixels_mut())
        .expect("failed to scale downloaded bitmap");
    led_strip
        .show()
        .await
        .expect("failed to show downloaded bitmap on LED strip");
    *current_bitmap_index = writable_index;

    info!(
        "applied downloaded image transfer {} ({} bytes, crc32=0x{:08x})",
        download.transfer_id, download.len, download.crc32
    );
}

async fn randomize_leds(led_strip: &mut WaveshareMatrix<'_>, rng: &Rng) {
    for pixel in led_strip.pixels_mut() {
        let value = rng.random();
        *pixel = RGB8 {
            r: (value & 0xFF) as u8,
            g: ((value >> 8) & 0xFF) as u8,
            b: ((value >> 16) & 0xFF) as u8,
        };
    }
    led_strip.show().await.expect("failed to show randomized LEDs");
}

async fn apply_command(
    led_strip: &mut WaveshareMatrix<'_>,
    bitmap_store: &impl BitmapStorage,
    current_bitmap_index: &mut usize,
    randomizing: &mut bool,
    frame: CommandFrame,
) {
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
            let bitmap_count = bitmap_store.bitmap_count();
            if bitmap_count == 0 {
                info!(
                    "ignoring NextImage command from transfer {}: no images",
                    frame.transfer_id
                );
                return;
            }

            *current_bitmap_index = (*current_bitmap_index + 1) % bitmap_count;
            render_bitmap_index(led_strip, bitmap_store, *current_bitmap_index).await;
            info!(
                "applied NextImage command from transfer {}: new_index={}",
                frame.transfer_id, *current_bitmap_index
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

    static DECODE_SCRATCH: StaticCell<[u8; WAVESHARE_RGB565_DECODE_SCRATCH_BYTES]> =
        StaticCell::new();
    let decode_scratch = DECODE_SCRATCH.init([0; WAVESHARE_RGB565_DECODE_SCRATCH_BYTES]);

    let mut bitmap_store = generated_image_storage();
    let mut current_bitmap_index = 0usize;
    let mut next_download_slot = 0usize;
    let mut randomizing = false;
    let rng = Rng::new();
    render_bitmap_index(&mut led_strip, &*bitmap_store, current_bitmap_index).await;
    info!("rendered built-in bitmap at startup");

    loop {
        let led_cmd = if randomizing {
            let refresh_period = led_strip.refresh_period();
            let delay = EmbassyDuration::from_micros(refresh_period.as_micros() as u64);
            match select(super::LED_COMMAND_CHANNEL.receive(), Timer::after(delay)).await {
                Either::First(cmd) => Some(cmd),
                Either::Second(_) => {
                    randomize_leds(&mut led_strip, &rng).await;
                    None
                }
            }
        } else {
            Some(super::LED_COMMAND_CHANNEL.receive().await)
        };

        let Some(led_cmd) = led_cmd else { continue };

        match led_cmd {
            LedCommand::Frame(frame) => {
                apply_command(
                    &mut led_strip,
                    &*bitmap_store,
                    &mut current_bitmap_index,
                    &mut randomizing,
                    frame,
                )
                .await;
            }
            LedCommand::Download(download) => match download.kind {
                DownloadKind::DisplayImage => apply_downloaded_image(
                    &mut led_strip,
                    &mut *bitmap_store,
                    &mut current_bitmap_index,
                    &mut next_download_slot,
                    decode_scratch,
                    &download,
                )
                .await,
                DownloadKind::OtaImage | DownloadKind::Video => {
                    info!(
                        "ignoring unsupported download kind on waveshare target: kind={:?} transfer_id={} bytes={}",
                        download.kind, download.transfer_id, download.len
                    );
                }
            },
        }
    }
}
