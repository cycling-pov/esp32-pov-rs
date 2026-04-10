use defmt::info;
#[cfg(feature = "sk9822-strip")]
use embassy_time::{Duration, Timer};
#[cfg(feature = "sk9822-strip")]
use esp_hal::{
    Blocking,
    gpio::Pin,
    spi::{
        Mode,
        master::{Config as SpiConfig, Instance as SpiInstance, Spi},
    },
    time::Rate,
};
#[cfg(feature = "sk9822-strip")]
use esp_spoke_firmware::led::{LedStrip, Sk9822Pins, Sk9822Strip};
#[cfg(feature = "sk9822-strip")]
use esp_spoke_firmware::networking;

#[cfg(feature = "sk9822-strip")]
const METRO_SK9822_LED_COUNT: usize = 30;

#[cfg(feature = "sk9822-strip")]
pub struct MetroSk9822Output<'d, SpiDevice>
where
    SpiDevice: SpiInstance + 'd,
{
    spi: SpiDevice,
    pins: Sk9822Pins<'d>,
}

#[cfg(feature = "sk9822-strip")]
impl<'d, SpiDevice> MetroSk9822Output<'d, SpiDevice>
where
    SpiDevice: SpiInstance + 'd,
{
    pub fn new(spi: SpiDevice, sk9822_clock: impl Pin + 'd, sk9822_data: impl Pin + 'd) -> Self {
        Self {
            spi,
            pins: Sk9822Pins::new(sk9822_clock, sk9822_data),
        }
    }
}

#[cfg(feature = "sk9822-strip")]
fn initialize_sk9822_output(strip: &mut Sk9822Strip<'_, METRO_SK9822_LED_COUNT>) {
    info!(
        "SK9822 strip ready: leds={}, timings={:?}",
        strip.led_count(),
        strip.timings()
    );

    strip.fill(smart_leds_trait::RGB8 { r: 255, g: 0, b: 0 });
    strip.show().expect("failed to update SK9822 strip");
}

#[cfg(feature = "sk9822-strip")]
pub async fn run_metro_output<'d, SpiDevice>(output: MetroSk9822Output<'d, SpiDevice>) -> !
where
    SpiDevice: SpiInstance + 'd,
{
    let spi: Spi<'_, Blocking> = Spi::new(
        output.spi,
        SpiConfig::default()
            .with_mode(Mode::_0)
            .with_frequency(Rate::from_mhz(1)),
    )
    .expect("failed to initialize SPI for SK9822");

    let mut strip = Sk9822Strip::<METRO_SK9822_LED_COUNT>::new(spi, output.pins);

    initialize_sk9822_output(&mut strip);
    info!("Adafruit Metro ESP32-S3 target active with SK9822 output + BLE ingest");

    loop {
        if let Some(command) = networking::try_receive_command() {
            info!(
                "Metro received command: transfer_id={} command={:?}. Command handling is not implemented for this output yet.",
                command.transfer_id, command.command
            );
        }

        if let Some(command) = networking::try_receive_download() {
            info!(
                "Metro received download: kind={:?} transfer_id={} bytes={} crc32=0x{:08x}. Payload application is not implemented for this output yet.",
                command.kind, command.transfer_id, command.len, command.crc32
            );
        }

        Timer::after(Duration::from_millis(25)).await;
    }
}

#[cfg(not(feature = "sk9822-strip"))]
pub fn initialize_metro_output() {
    info!("Adafruit Metro ESP32-S3 target active");
}
