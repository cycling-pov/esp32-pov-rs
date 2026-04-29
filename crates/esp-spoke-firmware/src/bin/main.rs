#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use defmt::info;
#[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
use defmt::warn;
use embassy_executor::Spawner;
#[cfg(feature = "heap-stats")]
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
#[cfg(feature = "sk9822-strip")]
use esp_spoke_firmware::led::Sk9822Pins;
use {esp_backtrace as _, esp_println as _};

extern crate alloc;

#[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
use embassy_futures::select::{Either, select};
use esp_hal::timer::timg::TimerGroup;

#[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
use esp_spoke_firmware::led;

#[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
use esp_spoke_firmware::led::LedCommand;
use esp_spoke_firmware::networking;
#[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
use esp_spoke_firmware::storage;

#[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
use embassy_embedded_hal::adapter::BlockingAsync;
#[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
use esp_storage::FlashStorage;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

const RECLAIMABLE_BOOTLOADER_BYTES: usize = 73744;
const ADDITIONAL_HEAP_BYTES: usize = 64 * 1024;
// COEX (simultaneous BLE + WiFi/ESP-NOW) requires extra heap on top.
#[cfg(feature = "coexistence")]
const COEX_HEAP_BYTES: usize = 64 * 1024;
#[cfg(feature = "heap-stats")]
#[embassy_executor::task]
async fn heap_stats_task() -> ! {
    loop {
        info!("heap stats:\n{}", esp_alloc::HEAP.stats());
        Timer::after(Duration::from_secs(30)).await;
    }
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // generator version: 1.2.0

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: RECLAIMABLE_BOOTLOADER_BYTES);
    esp_alloc::heap_allocator!(size: ADDITIONAL_HEAP_BYTES);
    // Extra heap required by COEX (running BLE and WiFi/ESP-NOW concurrently).
    #[cfg(feature = "coexistence")]
    esp_alloc::heap_allocator!(size: COEX_HEAP_BYTES);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    #[cfg(feature = "heap-stats")]
    spawner
        .spawn(heap_stats_task())
        .expect("failed to spawn heap stats task");

    info!("Embassy initialized!");

    networking::init(peripherals.WIFI, peripherals.BT, spawner).await;

    #[cfg(feature = "usb-serial")]
    {
        let usb = esp_hal::usb_serial_jtag::UsbSerialJtag::new(peripherals.USB_DEVICE).into_async();
        networking::start_usb_serial_backend(spawner, usb);
        info!("Serial backend initialized");
    }

    #[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
    let flash = BlockingAsync::new(FlashStorage::new(peripherals.FLASH));

    info!("Flash storage initialized");

    #[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
    storage::init(flash, spawner);

    info!(" storage initialized");

    #[cfg(feature = "waveshare-matrix")]
    led::init_waveshare(peripherals.RMT, peripherals.GPIO14, spawner);

    #[cfg(feature = "sk9822-strip")]
    led::init_sk9822(
        peripherals.SPI2,
        peripherals.DMA_CH0,
        Sk9822Pins::new(peripherals.GPIO12, peripherals.GPIO11),
        spawner,
    );

    info!("LED initialization completed");

    loop {
        // Forward networking events to the active LED task.
        #[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
        {
            info!("Loop: waiting for network event");

            match select(
                networking::receive_command(),
                networking::receive_download(),
            )
            .await
            {
                Either::First(Some(command)) => {
                    let transfer_id = command.transfer_id;
                    let command_kind = command.command;
                    if !led::try_send_led_command(LedCommand::Frame(command)) {
                        warn!(
                            "main:dropped frame transfer_id={} command={:?}",
                            transfer_id, command_kind
                        );
                    } else {
                        info!(
                            "main:forwarded frame transfer_id={} command={:?}",
                            transfer_id, command_kind
                        );
                    }
                }
                Either::Second(Some(download)) => {
                    let transfer_id = download.transfer_id;
                    let download_kind = download.kind;
                    let byte_len = download.len;
                    if !led::try_send_led_command(LedCommand::Download(download)) {
                        warn!(
                            "main:dropped download transfer_id={} kind={:?} bytes={}",
                            transfer_id, download_kind, byte_len
                        );
                    } else {
                        info!(
                            "main:forwarded download transfer_id={} kind={:?} bytes={}",
                            transfer_id, download_kind, byte_len
                        );
                    }
                }
                _ => {}
            }
        }
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0/examples
}
