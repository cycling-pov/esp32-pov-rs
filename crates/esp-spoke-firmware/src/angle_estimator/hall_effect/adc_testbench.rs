#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use defmt::info;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use {esp_backtrace as _, esp_println as _};

use esp_hal::analog;
use esp_hal::analog::adc::{Adc, AdcCalScheme, AdcChannel, AdcConfig, AdcPin, RegisterAccess};

extern crate alloc;

async fn read_adc_sample<'a, 'd, ADCX, PIN, CS>(
    adc: &'a mut Adc<'d, ADCX, esp_hal::Blocking>,
    pin: &'a mut AdcPin<PIN, ADCX, CS>,
) -> u16
where
    ADCX: RegisterAccess + 'd,
    PIN: AdcChannel,
    CS: AdcCalScheme<ADCX>,
{
    loop {
        if let Ok(sample) = adc.read_oneshot(pin) {
            break sample;
        }
        Timer::after(Duration::from_millis(1)).await;
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
async fn main(_spawner: Spawner) -> ! {
    // generator version: 1.2.0

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    info!("Embassy initialized!");

    // Configure two hall-sensor GPIOs as ADC1 inputs for periodic oneshot sampling.
    let mut adc1_config: AdcConfig<_> = AdcConfig::new();
    let mut hall_pin1 = adc1_config.enable_pin(peripherals.GPIO8, analog::adc::Attenuation::_11dB);
    let mut hall_pin2 = adc1_config.enable_pin(peripherals.GPIO4, analog::adc::Attenuation::_11dB);
    let mut status_resistor =
        adc1_config.enable_pin(peripherals.GPIO2, analog::adc::Attenuation::_0dB);
    let mut adc1 = Adc::new(peripherals.ADC1, adc1_config);

    const SAMPLE_FREQ_HZ: u64 = 20;
    const SAMPLE_INTERVAL: Duration = Duration::from_millis(1000 / SAMPLE_FREQ_HZ);

    let mut last_iter = embassy_time::Instant::now();

    loop {
        let now = embassy_time::Instant::now();
        let elapsed_us = now.duration_since(last_iter).as_micros();
        last_iter = now;
        info!("-------------------------- iter_us={=u64}", elapsed_us);

        let raw1 = read_adc_sample(&mut adc1, &mut hall_pin1).await;
        let raw2 = read_adc_sample(&mut adc1, &mut hall_pin2).await;
        let raw_status = read_adc_sample(&mut adc1, &mut status_resistor).await;

        info!("monitor1 sample: raw={=u16}", raw1);
        info!("monitor2 sample: raw={=u16}", raw2);
        info!("status resistor sample: raw={=u16}", raw_status);

        Timer::after(SAMPLE_INTERVAL).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0/examples
}
