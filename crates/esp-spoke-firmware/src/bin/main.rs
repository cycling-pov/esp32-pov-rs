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
#[cfg(all(feature = "sk9822-strip", feature = "imu-spin"))]
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
#[cfg(all(feature = "sk9822-strip", feature = "imu-spin"))]
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
#[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
use embassy_time::Duration;
#[cfg(any(
    feature = "heap-stats",
    all(
        feature = "adc",
        any(feature = "waveshare-matrix", feature = "sk9822-strip")
    )
))]
use embassy_time::Timer;
use esp_hal::clock::CpuClock;
#[cfg(all(
    feature = "sk9822-strip",
    feature = "imu-spin",
    feature = "hybrid-angle-estimator"
))]
use esp_spoke_firmware::angle_estimator::hybrid_dual_spin_estimator_task;
#[cfg(all(
    feature = "sk9822-strip",
    feature = "imu-spin",
    feature = "hybrid-angle-estimator"
))]
use esp_spoke_firmware::angle_estimator::imu::imu_spin_rate_publisher_task;
#[cfg(all(
    feature = "sk9822-strip",
    feature = "imu-spin",
    not(feature = "hybrid-angle-estimator")
))]
use esp_spoke_firmware::angle_estimator::imu_dual_spin_estimator_task;
#[cfg(feature = "sk9822-strip")]
use esp_spoke_firmware::angle_estimator::new_shared_spin_state;
#[cfg(feature = "sk9822-strip")]
use esp_spoke_firmware::led::Sk9822Pins;
#[cfg(feature = "sk9822-strip")]
use static_cell::StaticCell;
use {esp_backtrace as _, esp_println as _};

extern crate alloc;

#[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
use embassy_futures::select::{Either, select};
use esp_hal::timer::timg::TimerGroup;

#[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
use esp_spoke_firmware::led;

#[cfg(feature = "adc")]
use esp_spoke_firmware::adc;
#[cfg(feature = "imu-spin")]
use esp_spoke_firmware::angle_estimator::ImuCalibrationState;
#[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
use esp_spoke_firmware::led::LedCommand;
use esp_spoke_firmware::networking;
#[cfg(any(feature = "pushbutton-1", feature = "pushbutton-2"))]
use esp_spoke_firmware::pushbutton;
#[cfg(any(feature = "pushbutton-1", feature = "pushbutton-2"))]
use esp_spoke_firmware::pushbutton::ButtonId;
#[cfg(all(feature = "status-led", not(feature = "waveshare-matrix")))]
use esp_spoke_firmware::status_led::{self, StatusLedRequest};
#[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
use esp_spoke_firmware::storage;
#[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
use esp_spoke_firmware::storage::config::SensorConfig;

#[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
use pov_proto::transfer::DownloadKind;
#[cfg(all(
    feature = "adc",
    any(feature = "waveshare-matrix", feature = "sk9822-strip")
))]
use pov_proto::transfer::{AdcDevice as WireAdcDevice, AdcSample as WireAdcSample};
#[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
use pov_proto::transfer::{
    Packet, ResponseFrame, SpokeCommand, SpokeResponse, StorageStats as WireStorageStats,
    encode_packet,
};

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

const RECLAIMABLE_BOOTLOADER_BYTES: usize = 73744;
const ADDITIONAL_HEAP_BYTES: usize = 64 * 1024;
// COEX (simultaneous BLE + WiFi/ESP-NOW) requires extra heap on top.
#[cfg(feature = "coexistence")]
const COEX_HEAP_BYTES: usize = 56 * 1024;

#[cfg(all(
    feature = "adc",
    any(feature = "waveshare-matrix", feature = "sk9822-strip")
))]
fn wire_adc_device_to_local(device: WireAdcDevice) -> adc::AdcDevice {
    match device {
        WireAdcDevice::BoardRev => adc::AdcDevice::BoardRev,
        WireAdcDevice::HallEffectSensor2 => adc::AdcDevice::HallEffectSensor2,
        WireAdcDevice::BatteryVoltage => adc::AdcDevice::BatteryVoltage,
        WireAdcDevice::HallEffectSensor1 => adc::AdcDevice::HallEffectSensor1,
    }
}

#[cfg(all(
    feature = "adc",
    any(feature = "waveshare-matrix", feature = "sk9822-strip")
))]
fn local_adc_device_to_wire(device: adc::AdcDevice) -> WireAdcDevice {
    match device {
        adc::AdcDevice::BoardRev => WireAdcDevice::BoardRev,
        adc::AdcDevice::HallEffectSensor2 => WireAdcDevice::HallEffectSensor2,
        adc::AdcDevice::BatteryVoltage => WireAdcDevice::BatteryVoltage,
        adc::AdcDevice::HallEffectSensor1 => WireAdcDevice::HallEffectSensor1,
    }
}
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
    chunk_count: u16,
}

#[cfg(feature = "i2c-1")]
struct I2CConfig<'d> {
    pub sda: esp_hal::gpio::AnyPin<'d>,
    pub scl: esp_hal::gpio::AnyPin<'d>,
}

#[cfg(all(feature = "sk9822-strip", feature = "imu-spin"))]
type SharedI2cBus = Mutex<NoopRawMutex, esp_hal::i2c::master::I2c<'static, esp_hal::Async>>;

#[cfg(all(feature = "imu-spin", feature = "board-v1", feature = "board-test-rig"))]
compile_error!("features `board-v1` and `board-test-rig` are mutually exclusive");

#[cfg(all(
    feature = "imu-spin",
    not(any(feature = "board-v1", feature = "board-test-rig"))
))]
compile_error!("feature `imu-spin` requires either `board-v1` or `board-test-rig`");

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
    let sw_int =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let sw_int1 = sw_int.software_interrupt1;
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);
    info!("Embassy initialized!");

    #[cfg(feature = "heap-stats")]
    spawner.spawn(heap_stats_task().unwrap());

    networking::init(peripherals.WIFI, peripherals.BT, spawner).await;

    #[cfg(feature = "usb-serial")]
    {
        let usb = esp_hal::usb_serial_jtag::UsbSerialJtag::new(peripherals.USB_DEVICE).into_async();
        networking::start_usb_serial_backend(spawner, usb);
        info!("Serial backend initialized");
    }

    #[cfg(any(
        feature = "waveshare-matrix",
        feature = "sk9822-strip",
        feature = "adc"
    ))]
    {
        storage::init(peripherals.FLASH, spawner);
        info!("Flash storage initialized");
    }

    #[cfg(feature = "adc")]
    {
        adc::init(
            spawner,
            peripherals.ADC1,
            peripherals.GPIO2,
            peripherals.GPIO4,
            peripherals.GPIO5,
            peripherals.GPIO8,
        );
        info!("ADC monitor task initialized (GPIO2/GPIO4/GPIO5/GPIO8)");
    }

    #[cfg(feature = "pushbutton-1")]
    {
        spawner.spawn(
            pushbutton::button_input_task(peripherals.GPIO7.into(), ButtonId::Button1).unwrap(),
        );
        info!("Pushbutton-1 initialized on GPIO6");
    }

    #[cfg(feature = "pushbutton-2")]
    {
        spawner.spawn(
            pushbutton::button_input_task(peripherals.GPIO6.into(), ButtonId::Button2).unwrap(),
        );
        info!("Pushbutton-2 initialized on GPIO7");
    }

    #[cfg(all(feature = "status-led", not(feature = "waveshare-matrix")))]
    {
        status_led::init(peripherals.GPIO46, spawner);
        info!("Status LED initialized on GPIO46");
    }

    #[cfg(all(feature = "waveshare-matrix", not(feature = "sk9822-strip")))]
    {
        use esp_hal::system::Stack;
        use static_cell::StaticCell;

        static APP_CORE_STACK: StaticCell<Stack<65536>> = StaticCell::new();
        let rmt = peripherals.RMT;
        let gpio14 = peripherals.GPIO14;

        esp_rtos::start_second_core(
            peripherals.CPU_CTRL,
            sw_int1,
            APP_CORE_STACK.init(Stack::new()),
            move || {
                static CORE1_EXECUTOR: StaticCell<esp_rtos::embassy::Executor> = StaticCell::new();
                CORE1_EXECUTOR
                    .init(esp_rtos::embassy::Executor::new())
                    .run(|spawner| {
                        led::init_waveshare(rmt, gpio14, spawner);
                    });
            },
        );
    }

    #[cfg(feature = "imu-spin")]
    let i2c0 = peripherals.I2C0;

    #[cfg(all(
        feature = "imu-spin",
        feature = "board-v1",
        not(feature = "board-test-rig")
    ))]
    let i2c_config = I2CConfig {
        sda: peripherals.GPIO47.into(),
        scl: peripherals.GPIO48.into(),
    };

    #[cfg(all(
        feature = "imu-spin",
        feature = "board-test-rig",
        not(feature = "board-v1")
    ))]
    let i2c_config = I2CConfig {
        sda: peripherals.GPIO6.into(),
        scl: peripherals.GPIO5.into(),
    };

    #[cfg(feature = "sk9822-strip")]
    {
        use esp_hal::system::Stack;
        use pov_algs::Angle;

        static SPIN_STATE_0: StaticCell<esp_spoke_firmware::angle_estimator::SharedSpinState> =
            StaticCell::new();
        static SPIN_STATE_1: StaticCell<esp_spoke_firmware::angle_estimator::SharedSpinState> =
            StaticCell::new();

        let sensor_config = storage::get_sensor_config().await;
        let _hall_offset_0 = Angle::from_degrees(sensor_config.hall_offset_0_degrees);
        let _hall_offset_1 = Angle::from_degrees(sensor_config.hall_offset_1_degrees);
        #[cfg(all(feature = "imu-spin", not(feature = "hybrid-angle-estimator")))]
        let imu_offset_degrees = sensor_config.imu_offset_degrees;

        // Coerce &'static mut to &'static (shared, Copy) so the same reference
        // can be passed to both init_sk9822_dual and the core-1 tasks.
        let spin0: &'static esp_spoke_firmware::angle_estimator::SharedSpinState =
            SPIN_STATE_0.init(new_shared_spin_state());
        let spin1: &'static esp_spoke_firmware::angle_estimator::SharedSpinState =
            SPIN_STATE_1.init(new_shared_spin_state());

        let (dual, shared_bitmap) = led::init_sk9822_dual(
            peripherals.SPI2,
            peripherals.DMA_CH0,
            Sk9822Pins::new(peripherals.GPIO12, peripherals.GPIO11),
            peripherals.SPI3,
            peripherals.DMA_CH1,
            Sk9822Pins::new(peripherals.GPIO40, peripherals.GPIO41),
            spin0,
            spin1,
        );

        spawner.spawn(led::pov_command_task(shared_bitmap).unwrap());

        static APP_CORE_STACK: StaticCell<Stack<65536>> = StaticCell::new();

        // SAFETY: `dual` is exclusively owned; core 0 never accesses it again
        // after start_second_core. See `PovDualStrip`'s Send impl.
        esp_rtos::start_second_core(
            peripherals.CPU_CTRL,
            sw_int1,
            APP_CORE_STACK.init(Stack::new()),
            move || {
                static CORE1_EXECUTOR: StaticCell<esp_rtos::embassy::Executor> = StaticCell::new();
                #[cfg(feature = "imu-spin")]
                static I2C_BUS: StaticCell<SharedI2cBus> = StaticCell::new();
                CORE1_EXECUTOR
                    .init(esp_rtos::embassy::Executor::new())
                    .run(|spawner| {
                        spawner.spawn(led::pov_render_task(dual, shared_bitmap).unwrap());
                        #[cfg(feature = "imu-spin")]
                        let i2c = I2C_BUS.init(Mutex::new(
                            esp_hal::i2c::master::I2c::new(
                                i2c0,
                                esp_hal::i2c::master::Config::default()
                                    .with_frequency(esp_hal::time::Rate::from_khz(400)),
                            )
                            .expect("failed to initialize I2C0")
                            .with_sda(i2c_config.sda)
                            .with_scl(i2c_config.scl)
                            .into_async(),
                        ));
                        #[cfg(feature = "hybrid-angle-estimator")]
                        spawner.spawn(imu_spin_rate_publisher_task(I2cDevice::new(i2c)).unwrap());
                        #[cfg(feature = "hybrid-angle-estimator")]
                        spawner.spawn(
                            hybrid_dual_spin_estimator_task(
                                spin0,
                                spin1,
                                sensor_config.hall_offset_0_degrees,
                                sensor_config.hall_offset_1_degrees,
                            )
                            .unwrap(),
                        );
                        #[cfg(all(feature = "imu-spin", not(feature = "hybrid-angle-estimator")))]
                        spawner.spawn(
                            imu_dual_spin_estimator_task(
                                spin0,
                                spin1,
                                I2cDevice::new(i2c),
                                imu_offset_degrees,
                            )
                            .unwrap(),
                        );
                        #[cfg(all(not(feature = "mock-spin"), not(feature = "imu-spin")))]
                        spawner.spawn(
                            esp_spoke_firmware::angle_estimator::dual_spin_estimator_task(
                                spin0,
                                spin1,
                                _hall_offset_0,
                                _hall_offset_1,
                            )
                            .unwrap(),
                        );
                        #[cfg(feature = "mock-spin")]
                        spawner.spawn(
                            esp_spoke_firmware::angle_estimator::mock_dual_spin_estimator_task(
                                spin0, spin1,
                            )
                            .unwrap(),
                        );
                    });
            },
        );
    }

    info!("LED initialization completed");

    // Track the transfer currently being streamed to flash.
    #[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
    let mut active: Option<ActiveTransfer> = None;
    #[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
    let mut render_pause_held = false;
    #[cfg(all(
        feature = "adc",
        any(feature = "waveshare-matrix", feature = "sk9822-strip")
    ))]
    let mut adc_samples = adc::subscribe().expect("adc subscriber unavailable in main task");

    #[cfg(all(
        feature = "status-led",
        feature = "sk9822-strip",
        not(feature = "waveshare-matrix")
    ))]
    let mut desired_status = StatusLedRequest::BLINK_SLOW;

    #[cfg(feature = "imu-spin")]
    let mut imu_calibrating = true;

    #[cfg(feature = "imu-spin")]
    {
        if !led::try_send_led_command(LedCommand::SetDisplayEnabled(false)) {
            warn!("main:failed to enqueue initial display disable for imu calibration");
        }
    }

    #[cfg(all(
        feature = "status-led",
        feature = "sk9822-strip",
        not(feature = "waveshare-matrix"),
        feature = "imu-spin"
    ))]
    {
        let _ = status_led::try_send_request(StatusLedRequest::BLINK_FAST);
    }

    #[cfg(all(
        feature = "status-led",
        feature = "sk9822-strip",
        not(feature = "waveshare-matrix"),
        not(feature = "imu-spin")
    ))]
    {
        let _ = status_led::try_send_request(desired_status);
    }

    loop {
        // Forward networking events to the active LED task or storage layer.
        #[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
        {
            info!("Loop: waiting for network event");

            let mut command = None;
            let mut chunk = None;

            #[cfg(feature = "imu-spin")]
            match select(
                select(networking::receive_command(), networking::receive_chunk()),
                esp_spoke_firmware::angle_estimator::receive_imu_boot_calibration_state(),
            )
            .await
            {
                Either::First(Either::First(cmd)) => {
                    command = cmd;
                }
                Either::First(Either::Second(ch)) => {
                    chunk = ch;
                }
                Either::Second(state) => {
                    match state {
                        ImuCalibrationState::Calibrating => {
                            imu_calibrating = true;
                            if !led::try_send_led_command(LedCommand::SetDisplayEnabled(false)) {
                                warn!(
                                    "main:failed to enqueue display disable during imu calibration"
                                );
                            }
                        }
                        ImuCalibrationState::Ready => {
                            imu_calibrating = false;
                            if !led::try_send_led_command(LedCommand::SetDisplayEnabled(true)) {
                                warn!(
                                    "main:failed to enqueue display enable after imu calibration"
                                );
                            }
                        }
                    }

                    #[cfg(all(feature = "status-led", not(feature = "waveshare-matrix")))]
                    {
                        let effective = if imu_calibrating {
                            StatusLedRequest::BLINK_FAST
                        } else {
                            desired_status
                        };
                        let _ = status_led::try_send_request(effective);
                    }
                    continue;
                }
            }

            #[cfg(not(feature = "imu-spin"))]
            match select(networking::receive_command(), networking::receive_chunk()).await {
                Either::First(cmd) => {
                    command = cmd;
                }
                Either::Second(ch) => {
                    chunk = ch;
                }
            }

            if let Some(command) = command {
                let transfer_id = command.frame.transfer_id;
                let command_kind = command.frame.command;
                match command.frame.command {
                    SpokeCommand::SetActiveSlot { slot } => {
                        let slot_usize = match usize::try_from(slot) {
                            Ok(slot) => slot,
                            Err(_) => {
                                warn!(
                                    "main:reject SetActiveSlot transfer_id={} slot={} reason=out_of_range",
                                    transfer_id, slot
                                );
                                continue;
                            }
                        };
                        let image_ids = storage::list_image_ids().await.unwrap_or_default();

                        if !image_ids.contains(&slot_usize) {
                            warn!(
                                "main:reject SetActiveSlot transfer_id={} slot={} reason=out_of_range",
                                transfer_id, slot_usize
                            );
                            continue;
                        }

                        let mut set_slot_pause_held = false;
                        if !render_pause_held {
                            // Active-slot updates touch flash metadata and need the same
                            // render pause contract as other flash writes.
                            render_pause_held = true;
                            set_slot_pause_held = true;
                            if !led::pause_render_for_flash(Duration::from_millis(500)).await {
                                warn!(
                                    "main:render pause ack timeout before SetActiveSlot transfer_id={} slot={}",
                                    transfer_id, slot_usize
                                );
                            }
                        }

                        if storage::set_active_slot(slot_usize).await.is_err() {
                            warn!(
                                "main:failed SetActiveSlot transfer_id={} slot={}",
                                transfer_id, slot_usize
                            );
                            if set_slot_pause_held {
                                led::resume_render_after_flash();
                                render_pause_held = false;
                            }
                            continue;
                        }

                        if !led::try_send_led_command(LedCommand::LoadSlot(slot_usize)) {
                            warn!(
                                "main:failed to enqueue LoadSlot after SetActiveSlot transfer_id={} slot={}",
                                transfer_id, slot_usize
                            );
                        } else {
                            info!(
                                "main:applied SetActiveSlot transfer_id={} slot={}",
                                transfer_id, slot_usize
                            );

                            #[cfg(all(feature = "status-led", not(feature = "waveshare-matrix")))]
                            {
                                desired_status = StatusLedRequest::BLINK_SLOW;
                                let effective = {
                                    #[cfg(feature = "imu-spin")]
                                    {
                                        if imu_calibrating {
                                            StatusLedRequest::BLINK_FAST
                                        } else {
                                            desired_status
                                        }
                                    }
                                    #[cfg(not(feature = "imu-spin"))]
                                    {
                                        desired_status
                                    }
                                };
                                let _ = status_led::try_send_request(effective);
                            }
                        }

                        if set_slot_pause_held {
                            led::resume_render_after_flash();
                            render_pause_held = false;
                        }
                    }
                    SpokeCommand::ClearAllImages => {
                        if let Some(old) = active.take() {
                            info!(
                                "main:clear-all aborts active transfer {} in slot {}",
                                old.transfer_id, old.slot
                            );
                            storage::abort_slot(old.slot, old.chunk_count).await.ok();
                        }

                        if !render_pause_held {
                            // Keep the same pause contract as transfer writes: once
                            // requested, always resume afterward even on timeout.
                            render_pause_held = true;
                            if !led::pause_render_for_flash(Duration::from_millis(500)).await {
                                warn!(
                                    "main:render pause ack timeout before clear-all transfer_id={}",
                                    transfer_id
                                );
                            }
                        }

                        if storage::clear_all_images().await.is_err() {
                            warn!(
                                "main:failed to clear all images transfer_id={}",
                                transfer_id
                            );
                        } else {
                            info!("main:cleared all images transfer_id={}", transfer_id);
                        }

                        if render_pause_held {
                            led::resume_render_after_flash();
                            render_pause_held = false;
                        }

                        // Force display off after clearing storage.
                        if !led::try_send_led_command(LedCommand::Frame(
                            pov_proto::transfer::CommandFrame {
                                transfer_id,
                                command: SpokeCommand::DisplayOff,
                            },
                        )) {
                            warn!(
                                "main:failed to enqueue DisplayOff after clear transfer_id={}",
                                transfer_id
                            );
                        }

                        #[cfg(all(feature = "status-led", not(feature = "waveshare-matrix")))]
                        {
                            desired_status = StatusLedRequest::OFF;
                            let effective = {
                                #[cfg(feature = "imu-spin")]
                                {
                                    if imu_calibrating {
                                        StatusLedRequest::BLINK_FAST
                                    } else {
                                        desired_status
                                    }
                                }
                                #[cfg(not(feature = "imu-spin"))]
                                {
                                    desired_status
                                }
                            };
                            let _ = status_led::try_send_request(effective);
                        }
                    }
                    SpokeCommand::SetSensorOffsets {
                        hall_offset_0_degrees,
                        hall_offset_1_degrees,
                        imu_offset_degrees,
                    } => {
                        let result = storage::set_sensor_config(SensorConfig {
                            hall_offset_0_degrees,
                            hall_offset_1_degrees,
                            imu_offset_degrees,
                        })
                        .await;

                        if result.is_err() {
                            warn!(
                                "main:failed to persist sensor offsets transfer_id={}",
                                transfer_id
                            );
                        } else {
                            info!(
                                "main:persisted sensor offsets transfer_id={} reboot_required=true",
                                transfer_id
                            );
                        }
                    }
                    SpokeCommand::SetAdcMonitorSampleRateHz { hz } => {
                        let mut sample_rate_pause_held = false;
                        if !render_pause_held {
                            render_pause_held = true;
                            sample_rate_pause_held = true;
                            if !led::pause_render_for_flash(Duration::from_millis(500)).await {
                                warn!(
                                    "main:render pause ack timeout before SetAdcMonitorSampleRateHz transfer_id={} hz={}",
                                    transfer_id, hz
                                );
                            }
                        }

                        let result = storage::set_adc_monitor_sample_rate_hz(hz).await;

                        if result.is_err() {
                            warn!(
                                "main:failed to persist adc monitor sample rate transfer_id={} hz={}",
                                transfer_id, hz
                            );
                        } else {
                            info!(
                                "main:persisted adc monitor sample rate transfer_id={} hz={} reboot_required=true",
                                transfer_id, hz
                            );
                        }

                        if sample_rate_pause_held {
                            led::resume_render_after_flash();
                            render_pause_held = false;
                        }
                    }
                    SpokeCommand::SetHybridHallTriggerThreshold { threshold } => {
                        let mut threshold_pause_held = false;
                        if !render_pause_held {
                            render_pause_held = true;
                            threshold_pause_held = true;
                            if !led::pause_render_for_flash(Duration::from_millis(500)).await {
                                warn!(
                                    "main:render pause ack timeout before SetHybridHallTriggerThreshold transfer_id={} threshold={}",
                                    transfer_id, threshold
                                );
                            }
                        }

                        let result = storage::set_hybrid_hall_trigger_threshold(threshold).await;

                        if result.is_err() {
                            warn!(
                                "main:failed to persist hall trigger threshold transfer_id={} threshold={}",
                                transfer_id, threshold
                            );
                        } else {
                            info!(
                                "main:persisted hall trigger threshold transfer_id={} threshold={} reboot_required=true",
                                transfer_id, threshold
                            );
                        }

                        if threshold_pause_held {
                            led::resume_render_after_flash();
                            render_pause_held = false;
                        }
                    }
                    SpokeCommand::RequestStorageStats => {
                        let Some(source_peer) = command.source_peer else {
                            warn!(
                                "main:RequestStorageStats missing source peer transfer_id={}",
                                transfer_id
                            );
                            continue;
                        };

                        let stats = match storage::get_storage_stats().await {
                            Ok(stats) => stats,
                            Err(()) => {
                                warn!(
                                    "main:failed to get storage stats transfer_id={}",
                                    transfer_id
                                );
                                continue;
                            }
                        };

                        let mut out = [0u8; 256];
                        let encoded = match encode_packet(
                            Packet::Response(ResponseFrame {
                                transfer_id,
                                response: SpokeResponse::StorageStats(WireStorageStats {
                                    total_bytes: stats.total_bytes,
                                    used_bytes: stats.used_bytes,
                                    free_bytes: stats.free_bytes,
                                    image_count: stats.image_count as u32,
                                    active_image_id: stats.active_image_id.map(|v| v as u32),
                                }),
                            }),
                            &mut out,
                        ) {
                            Ok(n) => n,
                            Err(_) => {
                                warn!(
                                    "main:failed to encode storage stats response transfer_id={}",
                                    transfer_id
                                );
                                continue;
                            }
                        };

                        if networking::send_espnow_packet(source_peer, &out[..encoded])
                            .await
                            .is_err()
                        {
                            warn!(
                                "main:failed to send storage stats response transfer_id={}",
                                transfer_id
                            );
                        } else {
                            info!(
                                "main:sent storage stats response transfer_id={} to {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                                transfer_id,
                                source_peer[0],
                                source_peer[1],
                                source_peer[2],
                                source_peer[3],
                                source_peer[4],
                                source_peer[5]
                            );
                        }
                    }
                    #[cfg(feature = "adc")]
                    SpokeCommand::RequestAdcSample { device } => {
                        let Some(source_peer) = command.source_peer else {
                            warn!(
                                "main:RequestAdcSample missing source peer transfer_id={}",
                                transfer_id
                            );
                            continue;
                        };

                        while adc_samples.try_next_message_pure().is_some() {}

                        let requested_device = wire_adc_device_to_local(device);
                        adc::start_oneshot_mode(adc::AdcSelection::only(requested_device)).await;

                        let sample = loop {
                            match select(
                                adc_samples.next_message_pure(),
                                Timer::after(Duration::from_millis(250)),
                            )
                            .await
                            {
                                Either::First(sample) => {
                                    if sample.source != adc::AdcSampleSource::Oneshot
                                        || sample.device != requested_device
                                    {
                                        continue;
                                    }
                                    break sample;
                                }
                                Either::Second(_) => {
                                    warn!("main:adc sample timeout transfer_id={}", transfer_id);
                                    continue;
                                }
                            }
                        };

                        let mut out = [0u8; 256];
                        let encoded = match encode_packet(
                            Packet::Response(ResponseFrame {
                                transfer_id,
                                response: SpokeResponse::AdcSample(WireAdcSample {
                                    device: local_adc_device_to_wire(sample.device),
                                    raw: sample.raw,
                                }),
                            }),
                            &mut out,
                        ) {
                            Ok(n) => n,
                            Err(_) => {
                                warn!(
                                    "main:failed to encode adc sample response transfer_id={}",
                                    transfer_id
                                );
                                continue;
                            }
                        };

                        if networking::send_espnow_packet(source_peer, &out[..encoded])
                            .await
                            .is_err()
                        {
                            warn!(
                                "main:failed to send adc sample response transfer_id={}",
                                transfer_id
                            );
                        } else {
                            info!(
                                "main:sent adc sample response transfer_id={} raw={}",
                                transfer_id, sample.raw
                            );
                        }
                    }
                    #[cfg(not(feature = "adc"))]
                    SpokeCommand::RequestAdcSample { .. } => {
                        warn!(
                            "main:ignoring RequestAdcSample without adc feature transfer_id={}",
                            transfer_id
                        );
                    }
                    _ => {
                        #[cfg(all(feature = "status-led", not(feature = "waveshare-matrix")))]
                        {
                            match command_kind {
                                SpokeCommand::DisplayOff => {
                                    desired_status = StatusLedRequest::OFF;
                                }
                                SpokeCommand::RandomizeDisplay => {
                                    desired_status = StatusLedRequest::BLINK_FAST;
                                }
                                SpokeCommand::NextImage => {
                                    desired_status = StatusLedRequest::BLINK_SLOW;
                                }
                                _ => {}
                            }

                            let effective = {
                                #[cfg(feature = "imu-spin")]
                                {
                                    if imu_calibrating {
                                        StatusLedRequest::BLINK_FAST
                                    } else {
                                        desired_status
                                    }
                                }
                                #[cfg(not(feature = "imu-spin"))]
                                {
                                    desired_status
                                }
                            };
                            let _ = status_led::try_send_request(effective);
                        }

                        if !led::try_send_led_command(LedCommand::Frame(command.frame)) {
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
                }
            } else if let Some(chunk) = chunk {
                // Accept renderable payloads (static images and videos).
                // Keep ignoring non-renderable transfer kinds (e.g. OTA).
                if !matches!(chunk.kind, DownloadKind::DisplayImage | DownloadKind::Video) {
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
                    if !render_pause_held {
                        // Mark as held unconditionally: pause_render_for_flash always
                        // sets RENDER_PAUSE_REQUESTED, so resume_render_after_flash must
                        // always be called afterward — even if the ack times out — to
                        // avoid leaving the render task stuck in its IRAM spin loop.
                        render_pause_held = true;
                        if !led::pause_render_for_flash(Duration::from_millis(500)).await {
                            warn!(
                                "main:render pause ack timeout before transfer {}",
                                transfer_id
                            );
                        }
                    }

                    if let Some(old) = active.take() {
                        info!(
                            "main:new transfer {} aborts previous transfer {} in slot {}",
                            transfer_id, old.transfer_id, old.slot
                        );
                        storage::abort_slot(old.slot, old.chunk_count).await.ok();
                    }
                    match storage::begin_slot_write(chunk.total_len).await {
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
                                chunk_count: 0,
                            });
                        }
                        Err(()) => {
                            warn!(
                                "main:begin_slot_write failed for transfer_id={}",
                                transfer_id
                            );
                            if render_pause_held {
                                led::resume_render_after_flash();
                                render_pause_held = false;
                            }
                            // Drop this chunk; the next one will retry begin_slot_write.
                            continue;
                        }
                    }
                }

                if let Some(ref mut a) = active
                    && a.transfer_id == transfer_id
                {
                    let slot = a.slot;
                    let chunk_num = match u16::try_from(chunk.chunk_index) {
                        Ok(v) => v,
                        Err(_) => {
                            warn!(
                                "main:chunk index out of range slot={} idx={} transfer_id={}",
                                slot, chunk.chunk_index, transfer_id
                            );
                            continue;
                        }
                    };
                    let is_final = chunk.is_final;

                    if storage::write_slot_chunk(slot, chunk_num, &chunk.data)
                        .await
                        .is_err()
                    {
                        warn!(
                            "main:write_slot_chunk failed slot={} chunk={} transfer_id={}",
                            slot, chunk_num, transfer_id
                        );
                    } else {
                        a.chunk_count = a.chunk_count.saturating_add(1);
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
                            a.chunk_count,
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
                                } else {
                                    #[cfg(all(
                                        feature = "status-led",
                                        not(feature = "waveshare-matrix")
                                    ))]
                                    {
                                        desired_status = StatusLedRequest::BLINK_SLOW;
                                        let effective = {
                                            #[cfg(feature = "imu-spin")]
                                            {
                                                if imu_calibrating {
                                                    StatusLedRequest::BLINK_FAST
                                                } else {
                                                    desired_status
                                                }
                                            }
                                            #[cfg(not(feature = "imu-spin"))]
                                            {
                                                desired_status
                                            }
                                        };
                                        let _ = status_led::try_send_request(effective);
                                    }
                                }
                            }
                            Err(()) => {
                                warn!(
                                    "main:commit failed for transfer {} slot {} (CRC mismatch or header error)",
                                    a.transfer_id, a.slot
                                );
                            }
                        }

                        if render_pause_held {
                            led::resume_render_after_flash();
                            render_pause_held = false;
                        }
                    }
                }
            }
        }
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0/examples
}
