#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use core::cell::RefCell;

use critical_section::Mutex;
use defmt::info;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_spoke_firmware::angle_estimator::hall_effect::adc_monitor::{
    AdcMonitor, AdcSample, MonitorConfig, MonitorThreshold, SampleRate, wait_for_threshold0,
    wait_for_threshold1,
};
use {esp_backtrace as _, esp_println as _};

use esp_hal::analog;
use esp_hal::analog::adc::AdcConfig;

extern crate alloc;

static LAST_TICK_0: Mutex<RefCell<esp_hal::time::Duration>> =
    Mutex::new(RefCell::new(esp_hal::time::Duration::ZERO));
static LAST_TICK_1: Mutex<RefCell<esp_hal::time::Duration>> =
    Mutex::new(RefCell::new(esp_hal::time::Duration::ZERO));

#[embassy_executor::task]
async fn hall_monitor_task(start_time: esp_hal::time::Instant) {
    loop {
        let _ = wait_for_threshold0().await;

        critical_section::with(|cs| {
            let last_tick = LAST_TICK_0.replace(cs, start_time.elapsed());
            info!("tick 0: time since start = {=u64}", last_tick.as_millis());
        });
    }
}

#[embassy_executor::task]
async fn hall_monitor_task1(start_time: esp_hal::time::Instant) {
    loop {
        let _ = wait_for_threshold1().await;

        critical_section::with(|cs| {
            let last_tick = LAST_TICK_1.replace(cs, start_time.elapsed());
            info!("tick 1: time since start = {=u64}", last_tick.as_millis());
        });
    }
}

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // generator version: 1.2.0

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    info!("Embassy initialized!");

    // Get start time
    let start_time = esp_hal::time::Instant::now();

    // Configure a hall-sensor GPIO as ADC1 input and monitor it continuously.
    let mut adc1_config: AdcConfig<_> = AdcConfig::new();
    let hall_pin1 = adc1_config.enable_pin(peripherals.GPIO5, analog::adc::Attenuation::_0dB);

    let hall_pin2 = adc1_config.enable_pin(peripherals.GPIO6, analog::adc::Attenuation::_0dB);

    let config = MonitorConfig {
        attenuation: analog::adc::Attenuation::_0dB,
        threshold: MonitorThreshold {
            low: AdcSample::new(1800),
            high: AdcSample::new(3000),
        },
        sample_rate: SampleRate {
            timer_target: 200,
            sar_clk_div: 4,
        },
    };

    // Example for a single monitor channel.
    // let mut hall_monitor = AdcMonitor::new(peripherals.ADC1, hall_pin2, config.clone());

    let mut hall_monitor =
        AdcMonitor::new_dual(peripherals.ADC1, hall_pin1, hall_pin2, config, config);
    hall_monitor.start();

    spawner.spawn(hall_monitor_task(start_time).unwrap());
    spawner.spawn(hall_monitor_task1(start_time).unwrap());

    loop {
        info!("Hello world!");
        Timer::after(Duration::from_secs(30)).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0/examples
}
