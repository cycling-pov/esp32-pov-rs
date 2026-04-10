use bt_hci::controller::ExternalController;
use defmt::info;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
#[cfg(feature = "waveshare-matrix")]
use esp_hal::rmt::Rmt;
#[cfg(feature = "waveshare-matrix")]
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;
use esp_radio::wifi::{WifiController, WifiMode};
#[cfg(feature = "waveshare-matrix")]
use esp_spoke_firmware::led::{WaveshareMatrix, WaveshareMatrixPins};
use esp_spoke_firmware::networking;
use static_cell::StaticCell;
#[cfg(feature = "waveshare-matrix")]
mod image_wire;
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
// COEX (simultaneous BLE + WiFi/ESP-NOW) requires extra heap on top.
const COEX_HEAP_BYTES: usize = 64 * 1024;
#[cfg(feature = "heap-stats")]
#[embassy_executor::task]
async fn heap_stats_task() -> ! {
    loop {
        info!("heap stats:\n{}", esp_alloc::HEAP.stats());
        Timer::after(Duration::from_secs(30)).await;
    }
}

static BLE_RADIO_CONTROLLER: StaticCell<esp_radio::Controller<'static>> = StaticCell::new();
static WIFI_CONTROLLER: StaticCell<WifiController<'static>> = StaticCell::new();

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
pub async fn run(target: BoardTarget, spawner: Spawner) -> ! {
    // generator version: 1.2.0

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: TOTAL_HEAP_BYTES);
    // Extra heap required by COEX (running BLE and WiFi/ESP-NOW concurrently).
    esp_alloc::heap_allocator!(size: COEX_HEAP_BYTES);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    #[cfg(feature = "heap-stats")]
    spawner
        .spawn(heap_stats_task())
        .expect("failed to spawn heap stats task");

    info!("Embassy initialized!");

    let radio = BLE_RADIO_CONTROLLER
        .init(esp_radio::init().expect("failed to initialize radio controller"));

    // ---------- WiFi / ESP-NOW ---------------------------------------------------
    let (mut wifi_ctrl, interfaces) =
        esp_radio::wifi::new(radio, peripherals.WIFI, Default::default())
            .expect("failed to initialize WiFi");
    wifi_ctrl
        .set_mode(WifiMode::Sta)
        .expect("failed to set WiFi mode");
    info!("WiFi mode set to STA, starting WiFi...");
    wifi_ctrl.start_async().await.expect("failed to start WiFi");
    info!("WiFi started, configuring ESP-NOW...");
    let esp_now = interfaces.esp_now;

    // Set explicit WiFi channel to ensure spoke and bridge sync on the same channel.
    const ESPNOW_CHANNEL: u8 = 6;
    esp_now
        .set_channel(ESPNOW_CHANNEL)
        .expect("failed to set ESP-NOW channel");
    info!("ESP-NOW channel set to {}", ESPNOW_CHANNEL);

    // Keep `wifi_ctrl` alive — dropping it would call `esp_wifi_stop()`.
    let _wifi_ctrl = WIFI_CONTROLLER.init(wifi_ctrl);
    networking::esp_now::start_esp_now_backend(spawner, esp_now);

    // ---------- BLE --------------------------------------------------------------
    let ble_connector = BleConnector::new(radio, peripherals.BT, Default::default())
        .expect("failed to initialize BLE connector");
    let ble_controller: ExternalController<_, 1> = ExternalController::new(ble_connector);
    networking::ble::start_ble_backend(spawner, ble_controller);

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
                waveshare::run_waveshare_output(&mut led_strip).await;
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
                metro::run_metro_output(output).await;
            }

            #[cfg(not(feature = "sk9822-strip"))]
            {
                metro::initialize_metro_output();
            }
        }
    }

    #[cfg(not(feature = "sk9822-strip"))]
    loop {
        info!("Hello world!");
        embassy_time::Timer::after(embassy_time::Duration::from_secs(10)).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0/examples
}
