#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use bt_hci::controller::ExternalController;
use defmt::info;
use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Timer};
use esp_bridge_firmware::{
    ble_adv::{BleController, ble_adv_task},
    esp_now_broadcaster::esp_now_task,
    usb_serial::{ChunkMsg, usb_serial_task},
};
use esp_hal::{clock::CpuClock, timer::timg::TimerGroup, usb_serial_jtag::UsbSerialJtag};
use esp_radio::{
    ble::controller::BleConnector,
    esp_now::EspNow,
    wifi::{WifiController, WifiMode},
};
use static_cell::StaticCell;
use {esp_backtrace as _, esp_println as _};

extern crate alloc;

// This creates a default app-descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

/// Static channel capacity for each transport.
const CHAN_CAP: usize = 4;

static BLE_CHANNEL: Channel<CriticalSectionRawMutex, ChunkMsg, CHAN_CAP> = Channel::new();
static ESP_NOW_CHANNEL: Channel<CriticalSectionRawMutex, ChunkMsg, CHAN_CAP> = Channel::new();

/// The `esp_radio::Controller` is shared between WiFi (for ESP-NOW) and BLE.
/// It must be `'static` — use a `StaticCell` to obtain a `&'static` reference.
static RADIO: StaticCell<esp_radio::Controller<'static>> = StaticCell::new();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);
    // COEX needs extra heap.
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    info!("Embassy initialized!");

    // ---------- Radio init -------------------------------------------------------
    let radio_init = esp_radio::init().expect("Failed to initialize radio controller");
    // Stash in a StaticCell so we can hand out `&'static` references to both
    // the WiFi and BLE subsystems.
    let radio: &'static esp_radio::Controller<'static> = RADIO.init(radio_init);

    // ---------- WiFi / ESP-NOW ---------------------------------------------------
    let (mut wifi_ctrl, interfaces) =
        esp_radio::wifi::new(radio, peripherals.WIFI, Default::default())
            .expect("Failed to initialize WiFi");

    // ESP-NOW requires WiFi in STA mode and the driver started.
    wifi_ctrl
        .set_mode(WifiMode::Sta)
        .expect("Failed to set WiFi mode");
    info!("WiFi mode set to STA, starting WiFi...");
    wifi_ctrl.start_async().await.expect("Failed to start WiFi");
    info!("WiFi started, configuring ESP-NOW...");

    let esp_now: EspNow<'static> = interfaces.esp_now;

    // Set explicit WiFi channel to ensure spoke and bridge sync on the same channel.
    const ESPNOW_CHANNEL: u8 = 6;
    esp_now
        .set_channel(ESPNOW_CHANNEL)
        .expect("Failed to set ESP-NOW channel");
    info!("ESP-NOW channel set to {}", ESPNOW_CHANNEL);

    // Print diagnostics about ESP-NOW configuration
    match esp_now.version() {
        Ok(version) => info!("ESP-NOW version: {=u32}", version),
        Err(err) => info!("Failed to get ESP-NOW version: {:?}", err),
    }

    match esp_now.peer_count() {
        Ok(count) => {
            info!(
                "ESP-NOW peer count: total={=i32} encrypted={=i32}",
                count.total_count, count.encrypted_count
            );
        }
        Err(err) => info!("Failed to get peer count: {:?}", err),
    }

    info!("ESP-NOW ready to broadcast");

    // ---------- BLE --------------------------------------------------------------
    let ble_connector = BleConnector::new(radio, peripherals.BT, Default::default()).unwrap();
    let ble_ctrl: BleController = ExternalController::new(ble_connector);

    // ---------- USB Serial JTAG --------------------------------------------------
    let usb = UsbSerialJtag::new(peripherals.USB_DEVICE).into_async();

    // ---------- Spawn tasks -------------------------------------------------------
    spawner
        .spawn(usb_serial_task(
            usb,
            BLE_CHANNEL.sender(),
            ESP_NOW_CHANNEL.sender(),
        ))
        .expect("Failed to spawn usb_serial_task");

    spawner
        .spawn(ble_adv_task(ble_ctrl, BLE_CHANNEL.receiver()))
        .expect("Failed to spawn ble_adv_task");

    spawner
        .spawn(esp_now_task(esp_now, ESP_NOW_CHANNEL.receiver()))
        .expect("Failed to spawn esp_now_task");

    // Keep `wifi_ctrl` alive — dropping it would call `esp_wifi_stop()`.
    let _wifi_ctrl: WifiController<'static> = wifi_ctrl;

    info!("Wireless bridge running.");
    loop {
        info!("Heartbeat");
        Timer::after(Duration::from_secs(30)).await;
    }
}
