use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use pov_sender_core::{
    DownloadKind, DownloadRequest, EspNowDelivery, PolarEncodeOptions, SensorOffsets,
    SerialLinkConfig, SpokeCommand, Transport as CoreTransport, send_command, send_download,
    send_image, send_sensor_offsets, send_video,
};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Transport {
    Ble,
    Espnow,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum DownloadKindArg {
    DisplayImage,
    OtaImage,
    Video,
}

impl From<DownloadKindArg> for DownloadKind {
    fn from(value: DownloadKindArg) -> Self {
        match value {
            DownloadKindArg::DisplayImage => DownloadKind::DisplayImage,
            DownloadKindArg::OtaImage => DownloadKind::OtaImage,
            DownloadKindArg::Video => DownloadKind::Video,
        }
    }
}

impl From<Transport> for CoreTransport {
    fn from(value: Transport) -> Self {
        match value {
            Transport::Ble => CoreTransport::Ble,
            Transport::Espnow => CoreTransport::Espnow,
        }
    }
}

#[derive(Parser, Debug)]
#[command(about = "Send POV protocol messages over a wireless bridge adapter via USB-serial")]
struct Args {
    /// Serial port device (e.g. /dev/ttyUSB0 or COM3)
    #[arg(short, long)]
    port: String,

    /// Wireless transport the bridge should use
    #[arg(short, long, default_value = "espnow")]
    transport: Transport,

    /// Serial baud rate
    #[arg(short, long, default_value_t = 115_200)]
    baud: u32,

    /// Number of times to repeat each packet in random order for reliability
    #[arg(short, long, default_value_t = 1)]
    repeat: usize,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Send an image update.
    /// By default the image is resized to 64×64 and encoded in Cartesian format.
    /// With --polar the image is pre-converted to polar coordinates instead.
    SendImage {
        /// Path to the image file (PNG, JPEG, ...)
        #[arg(short, long)]
        image: PathBuf,

        /// Pre-convert the image to polar (radial × angular) coordinates before
        /// encoding. Requires --first-led-distance and --last-led-distance.
        #[arg(long, default_value_t = false)]
        polar: bool,

        /// Physical distance from hub center to LED 0 (innermost LED).
        /// Unit is arbitrary, but both distance arguments must use the same unit.
        #[arg(long)]
        first_led_distance: Option<f32>,

        /// Physical distance from hub center to LED 29 (outermost LED).
        /// Unit is arbitrary, but both distance arguments must use the same unit.
        #[arg(long)]
        last_led_distance: Option<f32>,
    },
    /// Send a GIF as a video payload.
    SendVideo {
        /// Path to a GIF file.
        #[arg(short, long)]
        gif: PathBuf,

        /// Pre-convert each frame to polar coordinates before encoding.
        #[arg(long, default_value_t = false)]
        polar: bool,

        /// Physical distance from hub center to LED 0 (innermost LED).
        #[arg(long)]
        first_led_distance: Option<f32>,

        /// Physical distance from hub center to LED 29 (outermost LED).
        #[arg(long)]
        last_led_distance: Option<f32>,
    },
    /// Send a raw file as a typed download payload.
    SendDownload {
        /// Payload kind for the receiver to route or apply.
        #[arg(short, long)]
        kind: DownloadKindArg,
        /// Path to the file to send without image re-encoding.
        #[arg(short, long)]
        file: PathBuf,
    },
    /// Turn the spoke display off.
    DisplayOff,
    /// Advance the spoke to the next stored image.
    NextImage,
    RandomizeDisplay,
    /// Persist hall and IMU offsets to nonvolatile storage.
    SetSensorOffsets {
        #[arg(long)]
        hall_offset_0_degrees: f32,
        #[arg(long)]
        hall_offset_1_degrees: f32,
        #[arg(long)]
        imu_offset_degrees: f32,
    },
    /// Persist the ADC monitor sample rate in hertz to nonvolatile storage.
    SetAdcMonitorSampleRateHz {
        #[arg(long)]
        hz: u16,
    },
    /// Persist the hybrid hall trigger threshold to nonvolatile storage.
    SetHybridHallTriggerThreshold {
        #[arg(long)]
        threshold: u16,
    },
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = SerialLinkConfig {
        port: args.port,
        baud: args.baud,
        transport: args.transport.into(),
        esp_now_delivery: EspNowDelivery::Broadcast,
        esp_now_retries: 0,
        repeat: args.repeat,
        inter_packet_delay_ms: 1_000,
    };

    let stats = match args.command {
        Command::SendImage {
            image,
            polar,
            first_led_distance,
            last_led_distance,
        } => {
            let polar_options = if polar {
                let first_led_distance =
                    first_led_distance.context("--polar requires --first-led-distance")?;
                let last_led_distance =
                    last_led_distance.context("--polar requires --last-led-distance")?;

                Some(PolarEncodeOptions {
                    first_led_distance,
                    last_led_distance,
                })
            } else {
                None
            };

            let stats = send_image(&config, &image, polar_options)?;
            println!(
                "Collected {} packets for image {:?}",
                stats.packet_count, image
            );
            stats
        }
        Command::SendVideo {
            gif,
            polar,
            first_led_distance,
            last_led_distance,
        } => {
            let polar_options = if polar {
                let first_led_distance =
                    first_led_distance.context("--polar requires --first-led-distance")?;
                let last_led_distance =
                    last_led_distance.context("--polar requires --last-led-distance")?;

                Some(PolarEncodeOptions {
                    first_led_distance,
                    last_led_distance,
                })
            } else {
                None
            };

            let stats = send_video(&config, &gif, polar_options)?;
            println!("Collected {} packets for GIF {:?}", stats.packet_count, gif);
            stats
        }
        Command::SendDownload { kind, file } => {
            let request = DownloadRequest {
                file_path: file.as_path(),
                kind: kind.into(),
            };
            let stats = send_download(&config, request)?;
            println!(
                "Collected {} packets for payload {:?}",
                stats.packet_count, file
            );
            stats
        }
        Command::DisplayOff => {
            let stats = send_command(&config, SpokeCommand::DisplayOff)?;
            println!("Collected command: DisplayOff");
            stats
        }
        Command::NextImage => {
            let stats = send_command(&config, SpokeCommand::NextImage)?;
            println!("Collected command: NextImage");
            stats
        }
        Command::RandomizeDisplay => {
            let stats = send_command(&config, SpokeCommand::RandomizeDisplay)?;
            println!("Collected command: RandomizeDisplay");
            stats
        }
        Command::SetSensorOffsets {
            hall_offset_0_degrees,
            hall_offset_1_degrees,
            imu_offset_degrees,
        } => {
            let stats = send_sensor_offsets(
                &config,
                SensorOffsets {
                    hall_offset_0_degrees,
                    hall_offset_1_degrees,
                    imu_offset_degrees,
                },
            )?;
            println!(
                "Collected command: SetSensorOffsets hall0={hall_offset_0_degrees} hall1={hall_offset_1_degrees} imu={imu_offset_degrees}"
            );
            stats
        }
        Command::SetAdcMonitorSampleRateHz { hz } => {
            let stats = send_command(&config, SpokeCommand::SetAdcMonitorSampleRateHz { hz })?;
            println!(
                "Collected command: SetAdcMonitorSampleRateHz hz={hz}. Reboot firmware to apply."
            );
            stats
        }
        Command::SetHybridHallTriggerThreshold { threshold } => {
            let stats = send_command(
                &config,
                SpokeCommand::SetHybridHallTriggerThreshold { threshold },
            )?;
            println!(
                "Collected command: SetHybridHallTriggerThreshold threshold={threshold}. Reboot firmware to apply."
            );
            stats
        }
    };

    println!(
        "✓ Sent {} packets as {} total transmissions",
        stats.packet_count, stats.total_transmissions
    );
    Ok(())
}
