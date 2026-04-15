#[cfg(feature = "sk9822-strip")]
mod sk9822_strip;
mod strip;
#[cfg(feature = "waveshare-matrix")]
mod waveshare_matrix;

use alloc::boxed::Box;
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
#[cfg(feature = "waveshare-matrix")]
use esp_hal::rmt::Rmt;
#[cfg(feature = "waveshare-matrix")]
use esp_hal::time::Rate;
use pov_proto::transfer::CommandFrame;

#[cfg(feature = "sk9822-strip")]
pub use sk9822_strip::{Sk9822Pins, Sk9822Strip};
pub use strip::{LedError, LedStrip, LedTimings};
#[cfg(feature = "waveshare-matrix")]
pub use waveshare_matrix::WaveshareMatrix;
#[cfg(feature = "waveshare-matrix")]
pub use waveshare_matrix::WaveshareMatrixPins;

use crate::networking::CompletedDownload;

/// Commands that can be sent to any LED output task.
pub enum LedCommand {
    Frame(CommandFrame),
    Download(Box<CompletedDownload>),
}

static LED_COMMAND_CHANNEL: Channel<CriticalSectionRawMutex, LedCommand, 4> = Channel::new();

/// Try to send a command to the active LED output task.
/// Returns `true` if the command was enqueued, `false` if the channel is full.
pub fn try_send_led_command(cmd: LedCommand) -> bool {
    LED_COMMAND_CHANNEL.try_send(cmd).is_ok()
}

#[cfg(feature = "waveshare-matrix")]
pub fn init_waveshare(
    rmt: esp_hal::peripherals::RMT<'static>,
    waveshare_pin: esp_hal::peripherals::GPIO14<'static>,
    spawner: Spawner,
) {
    let rmt = Rmt::new(rmt, Rate::from_mhz(80)).expect("failed to initialize RMT");
    let led_strip = WaveshareMatrix::new(rmt.channel0, WaveshareMatrixPins::new(waveshare_pin));
    spawner
        .spawn(waveshare_matrix::waveshare_matrix_task(led_strip))
        .expect("failed to spawn waveshare matrix task");
}

#[cfg(feature = "sk9822-strip")]
pub fn init_sk9822(
    spi: esp_hal::peripherals::SPI2<'static>,
    clock: esp_hal::peripherals::GPIO12<'static>,
    data: esp_hal::peripherals::GPIO11<'static>,
    spawner: Spawner,
) {
    use esp_hal::spi::Mode;
    use esp_hal::spi::master::{Config as SpiConfig, Spi};
    use esp_hal::time::Rate;

    let spi_bus = Spi::new(
        spi,
        SpiConfig::default()
            .with_mode(Mode::_0)
            .with_frequency(Rate::from_mhz(1)),
    )
    .expect("failed to initialize SPI for SK9822");

    let mut strip = Sk9822Strip::<{ sk9822_strip::SK9822_LED_COUNT }>::new(
        spi_bus,
        Sk9822Pins::new(clock, data),
    );

    strip.fill(smart_leds_trait::RGB8 { r: 255, g: 0, b: 0 });
    strip
        .show()
        .expect("failed to show initial red color on SK9822 strip");

    spawner
        .spawn(sk9822_strip::sk9822_strip_task(strip))
        .expect("failed to spawn SK9822 strip task");
}
