use defmt::info;
use esp_spoke_firmware::bitmap::{BitmapStorage, generated_image_storage};
use esp_spoke_firmware::led::{LedStrip, WaveshareMatrix};

pub fn initialize_waveshare_output(led_strip: &mut WaveshareMatrix<'_>) {
    info!(
        "LED strip ready: leds={}, timings={:?}",
        led_strip.led_count(),
        led_strip.timings()
    );

    let bitmap_store = generated_image_storage();
    let image_bitmap = bitmap_store.bitmap(0).expect("missing generated bitmap");
    let target_width = 8;
    let target_height = WaveshareMatrix::LED_COUNT / target_width;

    image_bitmap
        .scale_into(target_width, target_height, led_strip.pixels_mut())
        .expect("failed to scale generated bitmap");

    led_strip.show().expect("failed to update LED strip");
}
