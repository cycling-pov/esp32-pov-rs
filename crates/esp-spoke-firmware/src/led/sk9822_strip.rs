use defmt::{info, warn};
use embassy_futures::select::{Either, select};
use embassy_time::{Duration as EmbassyDuration, Timer};
use esp_hal::{
    Async,
    dma::DmaLoopBuf,
    gpio::{AnyPin, Pin},
    rng::Rng,
    spi::master::SpiDma,
};
use pov_proto::image::{DecodeMode, decode_into_rgb8};
use pov_proto::transfer::{DownloadKind, SpokeCommand};
use smart_leds_trait::RGB8;
use static_cell::StaticCell;

use crate::bitmap::{BitmapStorage, generated_swapping_storage};
use crate::led::{LedCommand, LedError, LedStrip, LedTimings};
use crate::networking::CompletedDownload;
use crate::storage;
use crate::storage::config::{ImageKind, ImageSlotState};

pub const SK9822_LED_COUNT: usize = 30;

const SK9822_RGB565_DECODE_SCRATCH_BYTES: usize = 1024 * 10;

async fn load_flash_slot(
    slot: usize,
    bitmap_store: &mut impl BitmapStorage,
    decode_scratch: &mut [u8],
) -> bool {
    let state = storage::get_slot_state(slot).await;
    if let ImageSlotState::Valid { .. } = state {
        match storage::read_slot_data(slot).await {
            Ok(img_bytes) => {
                if let Ok(mut writable) = bitmap_store.bitmap_mut(0) {
                    match decode_into_rgb8(
                        &img_bytes,
                        decode_scratch,
                        writable.pixels_mut(),
                        DecodeMode::ExactPixels,
                    ) {
                        Ok(_) => {
                            bitmap_store.activate_downloaded();
                            return true;
                        }
                        Err(err) => {
                            info!("sk9822:load flash slot {} decode error: {:?}", slot, err);
                        }
                    }
                }
            }
            Err(()) => info!("sk9822:load flash slot {} read error", slot),
        }
    }
    false
}

const SK9822_MAX_BRIGHTNESS: u8 = 31;
const SK9822_BRIGHTNESS_LIMIT_PERCENT: u8 = 5;
// SK9822 global brightness has 5 bits (0..31). 1/31 ~= 3.2%, 2/31 ~= 6.5%, so
// level 1 is the highest level that does not exceed 5%.
const SK9822_BRIGHTNESS: u8 =
    ((SK9822_MAX_BRIGHTNESS as u16 * SK9822_BRIGHTNESS_LIMIT_PERCENT as u16) / 100) as u8;
const SK9822_START_FRAME_BYTES: usize = 4;

const fn sk9822_end_frame_bytes(led_count: usize) -> usize {
    led_count.div_ceil(16)
}

pub const fn sk9822_frame_size(led_count: usize) -> usize {
    SK9822_START_FRAME_BYTES + (4 * led_count) + sk9822_end_frame_bytes(led_count)
}

pub struct Sk9822Pins<'d> {
    pub clock: AnyPin<'d>,
    pub data: AnyPin<'d>,
}

impl<'d> Sk9822Pins<'d> {
    pub fn new(clock: impl Pin + 'd, data: impl Pin + 'd) -> Self {
        Self {
            clock: clock.degrade(),
            data: data.degrade(),
        }
    }
}

pub struct Sk9822Strip<'d, const LED_COUNT: usize> {
    spi: Option<SpiDma<'d, Async>>,
    dma_buf: Option<DmaLoopBuf>,
    framebuffer: [RGB8; LED_COUNT],
}

impl<'d, const LED_COUNT: usize> Sk9822Strip<'d, LED_COUNT> {
    pub const LED_COUNT: usize = LED_COUNT;
    pub const TIMINGS: LedTimings = LedTimings::SK9822;

    pub fn new(spi: SpiDma<'d, Async>, dma_buf: DmaLoopBuf) -> Self {
        Self {
            spi: Some(spi),
            dma_buf: Some(dma_buf),
            framebuffer: [RGB8::default(); LED_COUNT],
        }
    }

    fn encode_framebuffer(&self, buf: &mut [u8]) {
        buf[..SK9822_START_FRAME_BYTES].fill(0);

        for (index, pixel) in self.framebuffer.iter().copied().enumerate() {
            let offset = SK9822_START_FRAME_BYTES + (index * 4);
            buf[offset] = 0b1110_0000 | SK9822_BRIGHTNESS;
            buf[offset + 1] = pixel.b;
            buf[offset + 2] = pixel.g;
            buf[offset + 3] = pixel.r;
        }

        let end_start = SK9822_START_FRAME_BYTES + (LED_COUNT * 4);
        let end_count = sk9822_end_frame_bytes(LED_COUNT);
        for byte in &mut buf[end_start..end_start + end_count] {
            *byte = 0xFF;
        }
    }
}

impl<const LED_COUNT: usize> LedStrip for Sk9822Strip<'_, LED_COUNT> {
    fn led_count(&self) -> usize {
        self.framebuffer.len()
    }

    fn timings(&self) -> LedTimings {
        Self::TIMINGS
    }

    fn pixels(&self) -> &[RGB8] {
        &self.framebuffer
    }

    fn pixels_mut(&mut self) -> &mut [RGB8] {
        &mut self.framebuffer
    }

    async fn show(&mut self) -> Result<(), LedError> {
        let mut dma_buf = self.dma_buf.take().expect("dma_buf missing");
        self.encode_framebuffer(&mut dma_buf);

        let frame_size = sk9822_frame_size(LED_COUNT);
        let spi = self.spi.take().expect("spi missing");
        let mut transfer = match spi.write(frame_size, dma_buf) {
            Ok(t) => t,
            Err((_, spi, dma_buf)) => {
                self.spi = Some(spi);
                self.dma_buf = Some(dma_buf);
                return Err(LedError::SpiWrite);
            }
        };

        transfer.wait_for_done().await;
        let (spi, dma_buf) = transfer.wait();
        self.spi = Some(spi);
        self.dma_buf = Some(dma_buf);
        Ok(())
    }
}

async fn render_active_bitmap(
    _led_strip: &mut Sk9822Strip<'_, SK9822_LED_COUNT>,
    bitmap_store: &impl BitmapStorage,
) {
    let _image_bitmap = bitmap_store.bitmap(0).expect("missing bitmap");

    info!("Bitmap rendering not implemented yet");
}

async fn apply_downloaded_image(
    led_strip: &mut Sk9822Strip<'_, SK9822_LED_COUNT>,
    bitmap_store: &mut impl BitmapStorage,
    current_display_slot: &mut Option<usize>,
    next_flash_slot: &mut usize,
    decode_scratch: &mut [u8],
    download: &CompletedDownload,
) {
    let metadata = bitmap_store.metadata();
    let pixel_count = metadata.pixel_count();

    info!(
        "sk9822:download start transfer_id={} kind={:?} bytes={} crc32=0x{:08x}",
        download.transfer_id, download.kind, download.len, download.crc32
    );

    let flash_slot = *next_flash_slot;
    *next_flash_slot = (*next_flash_slot + 1) % 2;
    info!(
        "sk9822:download flash_slot={} next_flash_slot={}",
        flash_slot, *next_flash_slot
    );

    let decoded = {
        let mut writable = bitmap_store
            .bitmap_mut(0)
            .expect("missing writable image slot");

        match decode_into_rgb8(
            download.payload(),
            decode_scratch,
            writable.pixels_mut(),
            DecodeMode::ExactPixels,
        ) {
            Ok(decoded) => decoded,
            Err(err) => {
                info!(
                    "ignoring transfer {}: failed to decode framed payload ({:?})",
                    download.transfer_id, err
                );
                return;
            }
        }
    };
    info!(
        "decoded transfer {} as {:?} ({} bytes, {} pixels)",
        download.transfer_id, decoded, download.len, pixel_count
    );

    bitmap_store.activate_downloaded();
    *current_display_slot = Some(flash_slot);

    // TODO: Use LED translation utility to map the bitmap to LED strip commands. For now, just set the LEDs to white
    led_strip.fill(smart_leds_trait::RGB8 {
        r: 255,
        g: 255,
        b: 255,
    });
    led_strip
        .show()
        .await
        .expect("failed to show downloaded bitmap on SK9822 strip");

    info!(
        "applied downloaded image transfer {} ({} bytes, crc32=0x{:08x})",
        download.transfer_id, download.len, download.crc32
    );

    // Persist the raw compressed payload to flash (two-phase commit).
    info!(
        "sk9822:flash persist begin transfer_id={} slot={}",
        download.transfer_id, flash_slot
    );
    if storage::set_slot_state(flash_slot, ImageSlotState::Writing)
        .await
        .is_ok()
    {
        match storage::write_slot_data(flash_slot, download.payload()).await {
            Ok(chunk_count) => {
                let state = ImageSlotState::Valid {
                    chunk_count,
                    total_bytes: download.len as u32,
                    kind: ImageKind::Static,
                    encoding: decoded,
                };
                if storage::set_slot_state(flash_slot, state).await.is_err() {
                    warn!("failed to mark flash slot {} as Valid", flash_slot);
                }
                if storage::set_active_slot(flash_slot as u8).await.is_err() {
                    warn!("failed to set active slot to {}", flash_slot);
                }
                info!(
                    "persisted transfer {} to flash slot {}",
                    download.transfer_id, flash_slot
                );
            }
            Err(()) => {
                warn!(
                    "failed to write flash slot {} for transfer {}",
                    flash_slot, download.transfer_id
                );
            }
        }
    } else {
        warn!("failed to mark flash slot {} as Writing", flash_slot);
    }
}

async fn apply_command(
    led_strip: &mut Sk9822Strip<'_, SK9822_LED_COUNT>,
    bitmap_store: &mut impl BitmapStorage,
    current_display_slot: &mut Option<usize>,
    decode_scratch: &mut [u8],
    randomizing: &mut bool,
    frame: pov_proto::transfer::CommandFrame,
) {
    info!(
        "sk9822:command received transfer_id={} command={:?}",
        frame.transfer_id, frame.command
    );

    match frame.command {
        SpokeCommand::DisplayOff => {
            *randomizing = false;
            led_strip.clear();
            led_strip
                .show()
                .await
                .expect("failed to clear SK9822 strip");
            info!(
                "applied DisplayOff command from transfer {}",
                frame.transfer_id
            );
        }
        SpokeCommand::NextImage => {
            *randomizing = false;
            // Cycle: None (built-in) → Some(0) → Some(1) → None → ...
            let next_slot = match *current_display_slot {
                None => Some(0usize),
                Some(0) => Some(1),
                Some(_) => None,
            };
            *current_display_slot = next_slot;
            match next_slot {
                None => {
                    bitmap_store.activate_builtin();
                    render_active_bitmap(led_strip, bitmap_store).await;
                }
                Some(slot) => {
                    if load_flash_slot(slot, bitmap_store, decode_scratch).await {
                        render_active_bitmap(led_strip, bitmap_store).await;
                    } else {
                        led_strip.clear();
                        led_strip
                            .show()
                            .await
                            .expect("failed to clear SK9822 strip");
                    }
                }
            }
            info!(
                "applied NextImage command from transfer {}: display_slot={:?}",
                frame.transfer_id, *current_display_slot
            );
        }
        SpokeCommand::RandomizeDisplay => {
            *randomizing = true;
            info!(
                "applied RandomizeDisplay command from transfer {}",
                frame.transfer_id
            );
        }
    }
}

#[embassy_executor::task]
pub async fn sk9822_strip_task(mut led_strip: Sk9822Strip<'static, SK9822_LED_COUNT>) -> ! {
    info!(
        "SK9822 strip ready: leds={}, timings={:?}",
        led_strip.led_count(),
        led_strip.timings()
    );

    static DECODE_SCRATCH: StaticCell<[u8; SK9822_RGB565_DECODE_SCRATCH_BYTES]> = StaticCell::new();
    let decode_scratch = DECODE_SCRATCH.init([0; SK9822_RGB565_DECODE_SCRATCH_BYTES]);

    let mut bitmap_store = generated_swapping_storage();
    let mut current_display_slot: Option<usize> = None;
    let mut next_flash_slot = 0usize;
    let mut randomizing = false;
    let rng = Rng::new();

    // Boot restore: load the active flash slot into the download buffer.
    let active_flash_slot = storage::get_active_slot().await;
    // Try the active slot first, fall back to the other slot.
    let slots_to_try: [usize; 2] = match active_flash_slot {
        Some(s) => [s as usize, (s as usize + 1) % 2],
        None => [0, 1],
    };
    for &slot in &slots_to_try {
        let state = storage::get_slot_state(slot).await;
        info!("sk9822:boot slot={} state={:?}", slot, state);
        if let ImageSlotState::Valid { .. } = state {
            if load_flash_slot(slot, &mut *bitmap_store, decode_scratch).await {
                info!(
                    "sk9822:boot restored flash slot {} into download buffer",
                    slot
                );
                current_display_slot = Some(slot);
                next_flash_slot = (slot + 1) % 2;
                break;
            } else {
                info!("sk9822:boot failed to load flash slot {}", slot);
            }
        }
    }
    if current_display_slot.is_some() {
        info!("sk9822:boot active image is downloaded from flash");
    } else {
        info!("sk9822:boot no valid flash image; starting with built-in");
    }

    render_active_bitmap(&mut led_strip, &*bitmap_store).await;
    info!("rendered bitmap at startup");

    loop {
        let led_cmd = if randomizing {
            let delay = EmbassyDuration::from_millis(10);
            match select(super::LED_COMMAND_CHANNEL.receive(), Timer::after(delay)).await {
                Either::First(cmd) => Some(cmd),
                Either::Second(_) => {
                    led_strip.randomize(&rng);
                    led_strip
                        .show()
                        .await
                        .expect("failed to show randomized SK9822 strip");
                    None
                }
            }
        } else {
            Some(super::LED_COMMAND_CHANNEL.receive().await)
        };

        let Some(led_cmd) = led_cmd else { continue };
        randomizing = false;

        match led_cmd {
            LedCommand::Frame(frame) => {
                info!(
                    "sk9822:loop handling frame transfer_id={} command={:?}",
                    frame.transfer_id, frame.command
                );
                apply_command(
                    &mut led_strip,
                    &mut *bitmap_store,
                    &mut current_display_slot,
                    decode_scratch,
                    &mut randomizing,
                    frame,
                )
                .await;
            }
            LedCommand::Download(download) => match download.kind {
                DownloadKind::DisplayImage => {
                    info!(
                        "sk9822:loop handling display download transfer_id={} bytes={}",
                        download.transfer_id, download.len
                    );
                    apply_downloaded_image(
                        &mut led_strip,
                        &mut *bitmap_store,
                        &mut current_display_slot,
                        &mut next_flash_slot,
                        decode_scratch,
                        &download,
                    )
                    .await
                }
                DownloadKind::OtaImage | DownloadKind::Video => {
                    info!(
                        "ignoring unsupported download kind on SK9822 target: kind={:?} transfer_id={} bytes={}",
                        download.kind, download.transfer_id, download.len
                    );
                }
            },
        }
    }
}
