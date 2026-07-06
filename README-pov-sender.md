# pov-sender
Host-side sender that transmits POV protocol frames to the bridge firmware over USB serial. The bridge then forwards payloads over BLE or ESP-NOW.

A CLI and GUI are provided to send packets using the same core logic in pov-sender-core.

# GUI
## 1. Run the tool

Builds must be executed from the tool's directory.
```sh
cd tools/pov-sender-gui
cargo run --release
```

# CLI

## 1. Build the tool

Builds must be executed from the tool's directory.

```sh
cd tools/pov-sender-cli
cargo build
```

The tool contains its own `rust-toolchain.toml` and `.cargo/config.toml` so it builds with stable Rust and uses the host target on Windows, macOS, or Linux.

## 2. Run common commands

From `tools/pov-sender-cli`:

```sh
cargo run -- --port <serial-port> display-off
cargo run -- --port <serial-port> next-image
cargo run -- --port <serial-port> send-image --image kirby.png
cargo run -- --port <serial-port> -t espnow send-image --image kirby.png
cargo run -- --port <serial-port> send-image --image kirby.png --polar --first-led-distance 18 --last-led-distance 72
```

In polar mode, LED radii are generated as evenly spaced points from the first LED distance to the last LED distance (inclusive). This naturally removes center-region samples between hub center and the first LED.

Send a typed raw payload:

```sh
cargo run -- --port <serial-port> send-download --kind ota-image --file firmware.bin
```

Supported transports:

- `ble`
- `espnow`

## 3. ESP-NOW broadcast vs stateful target mode (GUI)

In the sender GUI, ESP-NOW sends can now run in either:

- `broadcast` mode (legacy behavior, FF:FF:FF:FF:FF:FF)
- `stateful` mode (target a selected discovered peer MAC)

Use `Refresh Peers` to request the bridge's current discovered peer list, then
select a peer and set `Retries`.

Stateful mode retries failed ESP-NOW transmissions at the bridge layer to
improve reliability for commands like image/download sends and future response
flows (for example, storage stats queries).

### Storage stats command (GUI)

The Commands section includes a `Storage Stats` tab.

`Request Storage Stats` is only valid when all of the following are true:

- transport is `espnow`
- mode is `stateful`
- a target peer is selected

The spoke returns stats over ESP-NOW to the bridge, and the GUI displays:

- total bytes
- used bytes
- free bytes
- image count
- active image id

## 4. Reliability options

`--repeat` resends each generated packet multiple times in randomized order.

```sh
cargo run -- --port <serial-port> --repeat 3 send-image --image kirby.png
```

This improves delivery over lossy links or when BLE/ESP-NOW coexistence causes intermittent packet loss.

## 5. Serial port examples

- Windows: `COM5`
- Linux: `/dev/ttyUSB0`
- macOS: `/dev/cu.usbmodem*`
