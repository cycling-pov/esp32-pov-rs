#[cfg(feature = "sk9822-strip")]
mod sk9822_strip;
mod strip;
#[cfg(feature = "waveshare-matrix")]
mod waveshare_matrix;

use alloc::boxed::Box;

#[cfg(any(feature = "waveshare-matrix", feature = "sk9822-strip"))]
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
    dma_channel: esp_hal::peripherals::DMA_CH0<'static>,
    pins: Sk9822Pins<'static>,
    spawner: Spawner,
) {
    use esp_hal::dma::{DmaDescriptor, DmaLoopBuf};
    use esp_hal::spi::master::{Config as SpiConfig, Spi};
    use esp_hal::time::Rate;
    use static_cell::StaticCell;

    const FRAME_SIZE: usize = sk9822_strip::sk9822_frame_size(sk9822_strip::SK9822_LED_COUNT);

    static DMA_DESCRIPTOR: StaticCell<DmaDescriptor> = StaticCell::new();
    static DMA_BUF: StaticCell<[u8; FRAME_SIZE]> = StaticCell::new();

    let descriptor = DMA_DESCRIPTOR.init(DmaDescriptor::EMPTY);
    let buffer = DMA_BUF.init([0u8; FRAME_SIZE]);
    let dma_loop_buf =
        DmaLoopBuf::new(descriptor, buffer).expect("failed to create DMA loop buffer for SK9822");

    let spi_dma = Spi::new(spi, SpiConfig::default().with_frequency(Rate::from_mhz(30)))
        .expect("failed to initialize SPI for SK9822")
        .with_sck(pins.clock)
        .with_mosi(pins.data)
        .with_dma(dma_channel)
        .into_async();

    let strip = Sk9822Strip::<{ sk9822_strip::SK9822_LED_COUNT }>::new(spi_dma, dma_loop_buf);

    spawner
        .spawn(sk9822_strip::sk9822_strip_task(strip))
        .expect("failed to spawn SK9822 strip task");
}
