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
#[cfg(feature = "sk9822-strip")]
use metro::MetroSk9822Output;
#[cfg(feature = "waveshare-matrix")]
mod waveshare;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoardTarget {
    Waveshare,
    Metro,
}

const TOTAL_HEAP_BYTES: usize = 64 * 1024;

#[cfg(feature = "heap-stats")]
#[embassy_executor::task]
async fn heap_stats_task() -> ! {
    loop {
        info!("heap stats:\n{}", esp_alloc::HEAP.stats());
        Timer::after(Duration::from_secs(30)).await;
    }
}

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
pub async fn run(target: BoardTarget, spawner: Spawner) -> ! {
    // generator version: 1.2.0

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: TOTAL_HEAP_BYTES);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    #[cfg(feature = "heap-stats")]
    spawner
        .spawn(heap_stats_task())
        .expect("failed to spawn heap stats task");

    #[cfg(not(feature = "heap-stats"))]
    let _ = spawner;

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
            #[cfg(feature = "sk9822-strip")]
            {
                let output = MetroSk9822Output::new(
                    peripherals.SPI2,
                    peripherals.GPIO12,
                    peripherals.GPIO13,
                );
                metro::initialize_metro_output(output);
            }

            #[cfg(not(feature = "sk9822-strip"))]
            {
                metro::initialize_metro_output();
            }
        }
    }

    loop {
        info!("Hello world!");
        Timer::after(Duration::from_secs(10)).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0/examples
}
