#[cfg(feature = "sk9822-strip")]
mod pov_dual_strip;
#[cfg(feature = "sk9822-strip")]
mod sk9822_strip;
mod strip;
pub(crate) mod task_common;
#[cfg(feature = "waveshare-matrix")]
mod waveshare_matrix;

use defmt::{info, warn};

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
pub use pov_dual_strip::PovDualStrip;
#[cfg(feature = "sk9822-strip")]
pub use sk9822_strip::{Sk9822Pins, Sk9822Strip};

pub use strip::{LedBrightness, LedError, LedStrip, LedTimings};
#[cfg(feature = "waveshare-matrix")]
pub use waveshare_matrix::WaveshareMatrix;
#[cfg(feature = "waveshare-matrix")]
pub use waveshare_matrix::WaveshareMatrixPins;

/// Commands that can be sent to any LED output task.
pub enum LedCommand {
    Frame(CommandFrame),
    /// Load the image stored in the given flash slot and begin displaying it.
    LoadSlot(usize),
}

static LED_COMMAND_CHANNEL: Channel<CriticalSectionRawMutex, LedCommand, 4> = Channel::new();

/// Try to send a command to the active LED output task.
/// Returns `true` if the command was enqueued, `false` if the channel is full.
pub fn try_send_led_command(cmd: LedCommand) -> bool {
    match &cmd {
        LedCommand::Frame(frame) => {
            info!(
                "led:enqueue frame transfer_id={} command={:?}",
                frame.transfer_id, frame.command
            );
        }
        LedCommand::LoadSlot(slot) => {
            info!("led:enqueue load_slot slot={}", slot);
        }
    }

    if LED_COMMAND_CHANNEL.try_send(cmd).is_ok() {
        true
    } else {
        warn!("led:enqueue failed channel full");
        false
    }
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
#[allow(
    clippy::too_many_arguments,
    reason = "each SPI/DMA peripheral is a distinct type"
)]
pub fn init_sk9822_dual(
    spi0: esp_hal::peripherals::SPI2<'static>,
    dma_ch0: esp_hal::peripherals::DMA_CH0<'static>,
    pins0: Sk9822Pins<'static>,
    spi1: esp_hal::peripherals::SPI3<'static>,
    dma_ch1: esp_hal::peripherals::DMA_CH1<'static>,
    pins1: Sk9822Pins<'static>,
    spin_state0: &'static crate::angles::spin_estimator::SharedSpinState,
    spin_state1: &'static crate::angles::spin_estimator::SharedSpinState,
    spawner: Spawner,
) {
    use esp_hal::dma::{DmaDescriptor, DmaLoopBuf};
    use esp_hal::spi::master::{Config as SpiConfig, Spi};
    use esp_hal::time::Rate;
    use static_cell::StaticCell;

    const FRAME_SIZE: usize = sk9822_strip::sk9822_frame_size(sk9822_strip::SK9822_LED_COUNT);

    static DMA_DESCRIPTOR0: StaticCell<DmaDescriptor> = StaticCell::new();
    static DMA_BUF0: StaticCell<[u8; FRAME_SIZE]> = StaticCell::new();
    static DMA_DESCRIPTOR1: StaticCell<DmaDescriptor> = StaticCell::new();
    static DMA_BUF1: StaticCell<[u8; FRAME_SIZE]> = StaticCell::new();

    let dma_loop_buf0 = DmaLoopBuf::new(
        DMA_DESCRIPTOR0.init(DmaDescriptor::EMPTY),
        DMA_BUF0.init([0u8; FRAME_SIZE]),
    )
    .expect("failed to create DMA loop buffer for POV strip0");
    let dma_loop_buf1 = DmaLoopBuf::new(
        DMA_DESCRIPTOR1.init(DmaDescriptor::EMPTY),
        DMA_BUF1.init([0u8; FRAME_SIZE]),
    )
    .expect("failed to create DMA loop buffer for POV strip1");

    let spi_dma0 = Spi::new(
        spi0,
        SpiConfig::default().with_frequency(Rate::from_mhz(30)),
    )
    .expect("failed to initialize SPI0 for POV strip")
    .with_sck(pins0.clock)
    .with_mosi(pins0.data)
    .with_dma(dma_ch0)
    .into_async();
    let spi_dma1 = Spi::new(
        spi1,
        SpiConfig::default().with_frequency(Rate::from_mhz(30)),
    )
    .expect("failed to initialize SPI1 for POV strip")
    .with_sck(pins1.clock)
    .with_mosi(pins1.data)
    .with_dma(dma_ch1)
    .into_async();

    let strip0 = Sk9822Strip::<{ sk9822_strip::SK9822_LED_COUNT }>::new(spi_dma0, dma_loop_buf0);
    let strip1 = Sk9822Strip::<{ sk9822_strip::SK9822_LED_COUNT }>::new(spi_dma1, dma_loop_buf1);
    let dual = PovDualStrip::new(strip0, strip1, spin_state0, spin_state1);

    let shared_bitmap = pov_dual_strip::init_bitmap_store();
    spawner
        .spawn(pov_dual_strip::pov_render_task(dual, shared_bitmap))
        .expect("failed to spawn POV render task");
    spawner
        .spawn(pov_dual_strip::pov_command_task(shared_bitmap))
        .expect("failed to spawn POV command task");
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
