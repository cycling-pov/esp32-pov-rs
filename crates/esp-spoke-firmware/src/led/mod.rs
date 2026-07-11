#[cfg(feature = "sk9822-strip")]
mod pov_dual_strip;
#[cfg(feature = "sk9822-strip")]
mod sk9822_strip;
mod strip;
pub(crate) mod task_common;

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use defmt::{info, warn};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Timer};
use pov_proto::transfer::CommandFrame;

#[cfg(feature = "sk9822-strip")]
pub use pov_dual_strip::PovDualStrip;
#[cfg(feature = "sk9822-strip")]
pub use pov_dual_strip::{SharedBitmapMutex, pov_command_task, pov_render_task};
#[cfg(feature = "sk9822-strip")]
pub use sk9822_strip::{Sk9822Pins, Sk9822Strip};

pub use strip::{LedBrightness, LedError, LedStrip, LedTimings};

/// Commands that can be sent to any LED output task.
pub enum LedCommand {
    Frame(CommandFrame),
    /// Load the image stored in the given flash slot and begin displaying it.
    LoadSlot(usize),
    /// Enable or disable rendered LED output while keeping display state.
    SetDisplayEnabled(bool),
}

static LED_COMMAND_CHANNEL: Channel<CriticalSectionRawMutex, LedCommand, 4> = Channel::new();

pub(crate) const CORE1_FLASH_PAUSE_PARTICIPANTS: usize = 2;
const CORE1_FLASH_PAUSE_POLL_INTERVAL: Duration = Duration::from_millis(1);

pub(crate) static CORE1_FLASH_PAUSE_REQUESTED: AtomicBool = AtomicBool::new(false);
pub(crate) static CORE1_FLASH_PAUSED_COUNT: AtomicUsize = AtomicUsize::new(0);

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
        LedCommand::SetDisplayEnabled(enabled) => {
            info!("led:enqueue set_display_enabled={}", enabled);
        }
    }

    if LED_COMMAND_CHANNEL.try_send(cmd).is_ok() {
        true
    } else {
        warn!("led:enqueue failed channel full");
        false
    }
}

/// Request that all core1 tasks (render + spin estimator) pause before flash
/// mutations begin. Both tasks enter an IRAM-resident spin loop so core1 no
/// longer fetches from flash-backed ICache pages for the duration of the write.
/// Cache_Disable_ICache (called by ROM flash routines) is therefore harmless.
///
/// Returns `true` when all core1 tasks have acknowledged, or `false` on timeout.
/// For non-SK9822 builds this is a no-op that returns `true`.
pub async fn pause_render_for_flash(timeout: Duration) -> bool {
    #[cfg(feature = "sk9822-strip")]
    {
        CORE1_FLASH_PAUSED_COUNT.store(0, Ordering::Release);
        CORE1_FLASH_PAUSE_REQUESTED.store(true, Ordering::Release);

        let deadline = Instant::now() + timeout;
        loop {
            let paused_count = CORE1_FLASH_PAUSED_COUNT.load(Ordering::Acquire);
            if paused_count == CORE1_FLASH_PAUSE_PARTICIPANTS {
                // Both core1 tasks are now spinning in IRAM — flash writes are safe.
                info!("led:pause_render_for_flash: core1 in IRAM spin, flash safe");
                return true;
            }
            if Instant::now() >= deadline {
                warn!(
                    "led:pause_render_for_flash timeout paused_count={} expected={}",
                    paused_count, CORE1_FLASH_PAUSE_PARTICIPANTS
                );
                return false;
            }
            Timer::after(CORE1_FLASH_PAUSE_POLL_INTERVAL).await;
        }
    }

    #[cfg(not(feature = "sk9822-strip"))]
    {
        let _ = timeout;
        true
    }
}

/// Clear the pause request so all core1 tasks resume normal execution.
/// Clearing the flags causes the core1 IRAM spin loops to exit autonomously.
pub fn resume_render_after_flash() {
    #[cfg(feature = "sk9822-strip")]
    {
        CORE1_FLASH_PAUSE_REQUESTED.store(false, Ordering::Release);
        info!("led:resume_render_after_flash: all core1 tasks resumed");
    }
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
    spin_state0: &'static crate::angle_estimator::SharedSpinState,
    spin_state1: &'static crate::angle_estimator::SharedSpinState,
) -> (
    PovDualStrip<'static, { sk9822_strip::SK9822_LED_COUNT }>,
    &'static SharedBitmapMutex,
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
    (dual, shared_bitmap)
}
