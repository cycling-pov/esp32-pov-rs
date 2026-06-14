use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use pov_sender_core::{
    DownloadKind, DownloadRequest, PolarEncodeOptions, SerialLinkConfig, SpokeCommand,
    Transport as CoreTransport, send_command, send_download, send_image,
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
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = SerialLinkConfig {
        port: args.port,
        baud: args.baud,
        transport: args.transport.into(),
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
    };

    println!(
        "✓ Sent {} packets as {} total transmissions",
        stats.packet_count, stats.total_transmissions
    );
    Ok(())
}
