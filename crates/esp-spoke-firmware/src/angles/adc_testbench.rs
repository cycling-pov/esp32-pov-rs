#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use defmt::{info, unwrap};
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_spoke_firmware::angles::adc_monitor::{
    AdcMonitor, AdcSample, MonitorConfig, MonitorThreshold, SampleRate, ThresholdEvent,
    latest_sample0, latest_sample1, wait_for_threshold0, wait_for_threshold1,
};
use {esp_backtrace as _, esp_println as _};

use esp_hal::analog;
use esp_hal::analog::adc::AdcConfig;

extern crate alloc;

#[embassy_executor::task]
async fn hall_monitor_task() {
    loop {
        let event = wait_for_threshold0().await;
        let sample = latest_sample0();

        match event {
            ThresholdEvent::High => info!("hall0: high threshold hit, sample={=u16}", sample.raw()),
            ThresholdEvent::Low => info!("hall0: low threshold hit, sample={=u16}", sample.raw()),
            ThresholdEvent::Both => {
                info!("hall0: both thresholds hit, sample={=u16}", sample.raw())
            }
        }
    }
}

#[embassy_executor::task]
async fn hall_monitor_task1() {
    loop {
        let event = wait_for_threshold1().await;
        let sample = latest_sample1();

        match event {
            ThresholdEvent::High => info!("hall1: high threshold hit, sample={=u16}", sample.raw()),
            ThresholdEvent::Low => info!("hall1: low threshold hit, sample={=u16}", sample.raw()),
            ThresholdEvent::Both => {
                info!("hall1: both thresholds hit, sample={=u16}", sample.raw())
            }
        }
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
    esp_rtos::start(timg0.timer0);

    info!("Embassy initialized!");

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

    unwrap!(spawner.spawn(hall_monitor_task()));
    unwrap!(spawner.spawn(hall_monitor_task1()));

    loop {
        info!("Hello world!");
        Timer::after(Duration::from_secs(30)).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0/examples
}
