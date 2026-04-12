use defmt::info;
use embassy_time::{Duration, Timer};
use esp_spoke_firmware::bitmap::{BitmapStorage, generated_image_storage};
use esp_spoke_firmware::led::{LedStrip, WaveshareMatrix};
use esp_spoke_firmware::networking::{self, CompletedDownload};
use pov_proto::transfer::{CommandFrame, DownloadKind, SpokeCommand};
use static_cell::StaticCell;

use super::image_wire::{DecodeMode, decode_into_rgb8};

const WAVESHARE_RGB565_DECODE_SCRATCH_BYTES: usize = 1024 * 10;

static WAVESHARE_RGB565_DECODE_SCRATCH: StaticCell<[u8; WAVESHARE_RGB565_DECODE_SCRATCH_BYTES]> =
    StaticCell::new();
const DOWNLOADABLE_IMAGE_SLOTS: usize = 2;

fn render_bitmap_index(
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

    led_strip.show().expect("failed to update LED strip");
}

fn apply_downloaded_image(
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
        .expect("failed to show downloaded bitmap on LED strip");
    *current_bitmap_index = writable_index;

    info!(
        "applied downloaded image transfer {} ({} bytes, crc32=0x{:08x})",
        download.transfer_id, download.len, download.crc32
    );
}

fn apply_command(
    led_strip: &mut WaveshareMatrix<'_>,
    bitmap_store: &impl BitmapStorage,
    current_bitmap_index: &mut usize,
    frame: CommandFrame,
) {
    match frame.command {
        SpokeCommand::DisplayOff => {
            led_strip.clear();
            led_strip.show().expect("failed to clear LED strip");
            info!(
                "applied DisplayOff command from transfer {}",
                frame.transfer_id
            );
        }
        SpokeCommand::NextImage => {
            let bitmap_count = bitmap_store.bitmap_count();
            if bitmap_count == 0 {
                info!(
                    "ignoring NextImage command from transfer {}: no images",
                    frame.transfer_id
                );
                return;
            }

            *current_bitmap_index = (*current_bitmap_index + 1) % bitmap_count;
            render_bitmap_index(led_strip, bitmap_store, *current_bitmap_index);
            info!(
                "applied NextImage command from transfer {}: new_index={}",
                frame.transfer_id, *current_bitmap_index
            );
        }
    }
}

pub async fn run_waveshare_output(led_strip: &mut WaveshareMatrix<'_>) -> ! {
    info!(
        "LED strip ready: leds={}, timings={:?}",
        led_strip.led_count(),
        led_strip.timings()
    );

    let mut bitmap_store = generated_image_storage();
    let decode_scratch =
        WAVESHARE_RGB565_DECODE_SCRATCH.init([0; WAVESHARE_RGB565_DECODE_SCRATCH_BYTES]);
    let mut current_bitmap_index = 0usize;
    let mut next_download_slot = 0usize;
    render_bitmap_index(led_strip, &*bitmap_store, current_bitmap_index);
    info!("rendered built-in bitmap at startup");

    loop {
        if let Some(command) = networking::try_receive_command() {
            apply_command(
                led_strip,
                &*bitmap_store,
                &mut current_bitmap_index,
                command,
            );
        }

        if let Some(download) = networking::try_receive_download() {
            match download.kind {
                DownloadKind::DisplayImage => apply_downloaded_image(
                    led_strip,
                    &mut *bitmap_store,
                    &mut current_bitmap_index,
                    &mut next_download_slot,
                    decode_scratch,
                    &download,
                ),
                DownloadKind::OtaImage | DownloadKind::Video => {
                    info!(
                        "ignoring unsupported download kind on waveshare target: kind={:?} transfer_id={} bytes={}",
                        download.kind, download.transfer_id, download.len
                    );
                }
            }
        }

        Timer::after(Duration::from_millis(25)).await;
    }
}
