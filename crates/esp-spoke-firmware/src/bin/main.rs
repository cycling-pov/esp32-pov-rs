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
use esp_spoke_firmware::angles::{dual_spin_estimator_task, new_shared_spin_state};
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
use pov_proto::transfer::DownloadKind;

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

/// Tracks an in-progress streaming download being written to a flash slot.
#[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
struct ActiveTransfer {
    transfer_id: usize,
    slot: usize,
    kind: DownloadKind,
    expected_crc32: u32,
    total_len: u32,
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
    info!("Embassy initialized!");

    #[cfg(feature = "heap-stats")]
    spawner
        .spawn(heap_stats_task())
        .expect("failed to spawn heap stats task");

    networking::init(peripherals.WIFI, peripherals.BT, spawner).await;

    #[cfg(feature = "usb-serial")]
    {
        let usb = esp_hal::usb_serial_jtag::UsbSerialJtag::new(peripherals.USB_DEVICE).into_async();
        networking::start_usb_serial_backend(spawner, usb);
        info!("Serial backend initialized");
    }

    #[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
    {
        storage::init(peripherals.FLASH, spawner);
        info!("Flash storage initialized");
    }

    #[cfg(feature = "waveshare-matrix")]
    led::init_waveshare(peripherals.RMT, peripherals.GPIO14, spawner);

    #[cfg(feature = "sk9822-strip")]
    {
        use static_cell::StaticCell;

        static SPIN_STATE_0: StaticCell<esp_spoke_firmware::angles::SharedSpinState> =
            StaticCell::new();
        static SPIN_STATE_1: StaticCell<esp_spoke_firmware::angles::SharedSpinState> =
            StaticCell::new();

        let spin0 = SPIN_STATE_0.init(new_shared_spin_state());
        let spin1 = SPIN_STATE_1.init(new_shared_spin_state());

        spawner
            .spawn(dual_spin_estimator_task(spin0, spin1))
            .expect("failed to spawn dual spin estimator task");

        led::init_sk9822_dual(
            peripherals.SPI2,
            peripherals.DMA_CH0,
            Sk9822Pins::new(peripherals.GPIO12, peripherals.GPIO11),
            peripherals.SPI3,
            peripherals.DMA_CH1,
            Sk9822Pins::new(peripherals.GPIO10, peripherals.GPIO9),
            spin0,
            spin1,
            spawner,
        );
    }

    info!("LED initialization completed");

    // Track the transfer currently being streamed to flash.
    #[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
    let mut active: Option<ActiveTransfer> = None;

    loop {
        // Forward networking events to the active LED task or storage layer.
        #[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
        {
            info!("Loop: waiting for network event");

            match select(networking::receive_command(), networking::receive_chunk()).await {
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
                Either::Second(Some(chunk)) => {
                    // Only handle DisplayImage downloads; silently drop others.
                    if chunk.kind != DownloadKind::DisplayImage {
                        info!(
                            "main:ignoring non-display download kind={:?} transfer_id={}",
                            chunk.kind, chunk.transfer_id
                        );
                        continue;
                    }

                    let transfer_id = chunk.transfer_id;

                    // If a new transfer has started, abort the previous one and
                    // allocate a fresh flash slot.
                    if active.as_ref().is_none_or(|a| a.transfer_id != transfer_id) {
                        if let Some(old) = active.take() {
                            info!(
                                "main:new transfer {} aborts previous transfer {} in slot {}",
                                transfer_id, old.transfer_id, old.slot
                            );
                            storage::abort_slot(old.slot).await.ok();
                        }
                        match storage::begin_slot_write().await {
                            Ok(slot) => {
                                info!(
                                    "main:began slot write slot={} transfer_id={}",
                                    slot, transfer_id
                                );
                                active = Some(ActiveTransfer {
                                    transfer_id,
                                    slot,
                                    kind: chunk.kind,
                                    expected_crc32: chunk.expected_crc32,
                                    total_len: chunk.total_len,
                                });
                            }
                            Err(()) => {
                                warn!(
                                    "main:begin_slot_write failed for transfer_id={}",
                                    transfer_id
                                );
                                // Drop this chunk; the next one will retry begin_slot_write.
                                continue;
                            }
                        }
                    }

                    if let Some(ref a) = active
                        && a.transfer_id == transfer_id
                    {
                        let slot = a.slot;
                        let byte_offset = chunk.byte_offset;
                        let chunk_num = (byte_offset / storage::CHUNK_SIZE as u32) as u16;
                        let is_final = chunk.is_final;

                        if storage::write_slot_chunk(slot, chunk_num, &chunk.data)
                            .await
                            .is_err()
                        {
                            warn!(
                                "main:write_slot_chunk failed slot={} chunk={} transfer_id={}",
                                slot, chunk_num, transfer_id
                            );
                        }

                        if is_final {
                            let a = active.take().unwrap();
                            info!(
                                "main:committing slot={} transfer_id={} crc32={=u32:#010x} bytes={}",
                                a.slot, a.transfer_id, a.expected_crc32, a.total_len
                            );
                            match storage::commit_slot(
                                a.slot,
                                a.expected_crc32,
                                a.total_len,
                                a.kind,
                            )
                            .await
                            {
                                Ok(()) => {
                                    info!(
                                        "main:transfer {} committed to slot {}",
                                        a.transfer_id, a.slot
                                    );
                                    if !led::try_send_led_command(LedCommand::LoadSlot(a.slot)) {
                                        warn!(
                                            "main:dropped load_slot slot={} led channel full",
                                            a.slot
                                        );
                                    }
                                }
                                Err(()) => {
                                    warn!(
                                        "main:commit failed for transfer {} slot {} (CRC mismatch or header error)",
                                        a.transfer_id, a.slot
                                    );
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0/examples
}
