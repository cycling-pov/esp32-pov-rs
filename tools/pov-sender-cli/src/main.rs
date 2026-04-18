use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
    thread,
    time::Duration,
};

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use pov_proto::{
    bridge::{BridgeFrame, TransportSelector},
    image::encode_rgb888_to_wire,
    transfer::{encode_packet, ChunkIter, CommandFrame, DownloadKind, Packet, SpokeCommand},
};
use rand::seq::SliceRandom;
use serialport::SerialPort;

/// ESP-NOW 2.0 supports up to 1470-byte packets including protocol metadata.
/// Keep chunk payload lower so postcard-encoded transfer packets fit the MTU.
const ESPNOW_CHUNK_PAYLOAD_BYTES: usize = 1450;
/// BLE extended advertising caps the manufacturer-specific AD payload at ~250 bytes.
const BLE_CHUNK_PAYLOAD_BYTES: usize = 224;
/// Must be large enough to hold a postcard-encoded pov-proto packet whose
/// payload is up to ESPNOW_CHUNK_PAYLOAD_BYTES bytes (~1490 bytes max).
const SERIAL_TX_BUF_BYTES: usize = 1600;

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
    /// Send an image update (resized to 64x64).
    SendImage {
        /// Path to the image file (PNG, JPEG, ...)
        #[arg(short, long)]
        image: PathBuf,
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
    let max_chunk_payload = match args.transport {
        Transport::Espnow => ESPNOW_CHUNK_PAYLOAD_BYTES,
        Transport::Ble => BLE_CHUNK_PAYLOAD_BYTES,
    };

    let transport_selector = match args.transport {
        Transport::Ble => TransportSelector::BleExtAdv,
        Transport::Espnow => TransportSelector::EspNow,
    };

    // ---- Open serial port ----------------------------------------------------
    let mut port: Box<dyn SerialPort> = serialport::new(&args.port, args.baud)
        .timeout(Duration::from_secs(5))
        .open()
        .with_context(|| format!("Failed to open serial port {}", args.port))?;

    let mut chunk_buf = [0u8; SERIAL_TX_BUF_BYTES];

    // Collect all packets first
    let mut packets: Vec<Vec<u8>> = Vec::new();

    match args.command {
        Command::SendImage { image } => {
            let img =
                image::open(&image).with_context(|| format!("Failed to open image {:?}", image))?;
            let resized = img.resize_exact(64, 64, image::imageops::FilterType::Lanczos3);
            let pixels: Vec<u8> = resized.to_rgb8().into_raw();

            let wire_bytes = encode_rgb888_to_wire(&pixels).map_err(|e| {
                anyhow::anyhow!("Failed to encode image to pov-proto wire format: {:?}", e)
            })?;

            let iter = ChunkIter::new(
                &wire_bytes,
                DownloadKind::DisplayImage,
                1,
                max_chunk_payload,
            )
            .expect("Image payload too large for pov-proto transfer");

            for chunk in iter {
                let n = encode_packet(Packet::Download(chunk), &mut chunk_buf).map_err(|e| {
                    anyhow::anyhow!(
                        "encode_chunk failed: {:?}; payload_len={}, chunk_index={}, chunk_count={}, total_len={}, max_chunk_payload={}, wire_len={}",
                        e,
                        chunk.payload.len(),
                        chunk.chunk_index,
                        chunk.chunk_count,
                        chunk.total_len,
                        max_chunk_payload,
                        wire_bytes.len()
                    )
                })?;
                packets.push(chunk_buf[..n].to_vec());
            }

            println!("Collected {} chunks for image {:?}", packets.len(), image);
        }
        Command::SendDownload { kind, file } => {
            let payload = fs::read(&file)
                .with_context(|| format!("Failed to read payload file {:?}", file))?;

            let iter = ChunkIter::new(&payload, kind.into(), 1, max_chunk_payload)
                .expect("Download payload too large for pov-proto transfer");

            for chunk in iter {
                let n = encode_packet(Packet::Download(chunk), &mut chunk_buf).map_err(|e| {
                    anyhow::anyhow!(
                        "encode download failed: {:?}; payload_len={}, chunk_index={}, chunk_count={}, total_len={}, max_chunk_payload={}",
                        e,
                        chunk.payload.len(),
                        chunk.chunk_index,
                        chunk.chunk_count,
                        chunk.total_len,
                        max_chunk_payload,
                    )
                })?;
                packets.push(chunk_buf[..n].to_vec());
            }

            println!(
                "Collected {} chunks for {:?} payload {:?}",
                packets.len(),
                kind,
                file
            );
        }
        Command::DisplayOff => {
            let n = encode_packet(
                Packet::Command(CommandFrame {
                    transfer_id: 1,
                    command: SpokeCommand::DisplayOff,
                }),
                &mut chunk_buf,
            )
            .map_err(|e| anyhow::anyhow!("Failed to encode DisplayOff command: {:?}", e))?;
            packets.push(chunk_buf[..n].to_vec());
            println!("Collected command: DisplayOff");
        }
        Command::NextImage => {
            let n = encode_packet(
                Packet::Command(CommandFrame {
                    transfer_id: 1,
                    command: SpokeCommand::NextImage,
                }),
                &mut chunk_buf,
            )
            .map_err(|e| anyhow::anyhow!("Failed to encode NextImage command: {:?}", e))?;
            packets.push(chunk_buf[..n].to_vec());
            println!("Collected command: NextImage");
        }
        Command::RandomizeDisplay => {
            let n = encode_packet(
                Packet::Command(CommandFrame {
                    transfer_id: 1,
                    command: SpokeCommand::RandomizeDisplay,
                }),
                &mut chunk_buf,
            )
            .map_err(|e| anyhow::anyhow!("Failed to encode RandomizeDisplay command: {:?}", e))?;
            packets.push(chunk_buf[..n].to_vec());
            println!("Collected command: RandomizeDisplay");
        }
    }

    // Send packets with repetition and randomization
    let total_sends = packets.len() * args.repeat;
    println!(
        "Sending {} packets × {} repetitions = {} total transmissions",
        packets.len(),
        args.repeat,
        total_sends
    );

    for rep in 0..args.repeat {
        // Shuffle packets for this repetition
        let mut packet_indices: Vec<usize> = (0..packets.len()).collect();
        packet_indices.shuffle(&mut rand::thread_rng());

        for (i, &idx) in packet_indices.iter().enumerate() {
            let packet_num = rep * packets.len() + i + 1;
            print!(
                "\r[{}/{}] Sending packet repetition {}/{}...",
                packet_num,
                total_sends,
                rep + 1,
                args.repeat
            );
            let _ = io::stdout().flush();

            send_bridge_frame(&mut *port, transport_selector, &packets[idx])?;
        }
    }

    println!("\n✓ All {} transmissions sent", total_sends);
    Ok(())
}

fn send_bridge_frame(
    port: &mut dyn SerialPort,
    transport_selector: TransportSelector,
    payload: &[u8],
) -> anyhow::Result<()> {
    let frame = BridgeFrame {
        transport: transport_selector,
        payload,
    };

    let cobs_bytes = postcard::to_stdvec_cobs(&frame).context("postcard serialization failed")?;

    port.write_all(&cobs_bytes)
        .context("Failed to write to serial port")?;

    // Give the bridge time to process each frame before the next arrives.
    thread::sleep(Duration::from_millis(1000));

    Ok(())
}
