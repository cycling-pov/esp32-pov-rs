use defmt::info;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
#[cfg(feature = "waveshare-matrix")]
use esp_hal::rmt::Rmt;
#[cfg(feature = "waveshare-matrix")]
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
#[cfg(feature = "waveshare-matrix")]
use esp32_pov_rs::led::{WaveshareMatrix, WaveshareMatrixPins};
mod metro;
#[cfg(feature = "waveshare-matrix")]
mod waveshare;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoardTarget {
    Waveshare,
    Metro,
}

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
pub async fn run(target: BoardTarget, _spawner: Spawner) -> ! {
    // generator version: 1.2.0

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    info!("Embassy initialized!");

    match target {
        BoardTarget::Waveshare => {
            #[cfg(feature = "waveshare-matrix")]
            {
                let rmt = Rmt::new(peripherals.RMT, Rate::from_mhz(80))
                    .expect("failed to initialize RMT");
                let mut led_strip = WaveshareMatrix::new(
                    rmt.channel0,
                    WaveshareMatrixPins::new(peripherals.GPIO14),
                );
                waveshare::initialize_waveshare_output(&mut led_strip);
            }

            #[cfg(not(feature = "waveshare-matrix"))]
            panic!("Waveshare binary requires 'waveshare-matrix' feature");
        }
        BoardTarget::Metro => {
            metro::initialize_metro_output();
        }
    }

    loop {
        info!("Hello world!");
        Timer::after(Duration::from_secs(10)).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0/examples
}
