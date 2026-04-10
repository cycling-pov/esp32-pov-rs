# esp32-pov-rs
A persistence of vision embedded device for bike wheels. Contains several pieces of firmware and tools to support said project.

# Getting Started
This project targets the ESP32-S3 SOC using the esp-rs toolchain and utilizes the esp-hal, a native Rust HAL.

## 1. Install required tools

### Rust + esp-rs toolchain

1. Install Rust using the official instructions:

   - https://www.rust-lang.org/tools/install

2. Follow the esp-rs setup documentation for prerequisites and toolchain setup:

   - https://docs.espressif.com/projects/rust/book/getting-started/toolchain.html#xtensa-devices

      ```sh
      cargo install espup --locked
      espup install
      ```
   - Unix users will need to follow additional steps for environment variable setup. See the link above for details.

3. Restart your terminal after `espup install` so environment variables are loaded.

4. Install the hardware flashing tool (espflash)

    - https://docs.espressif.com/projects/rust/book/getting-started/tooling/espflash.html

      ```sh
      cargo install espflash --locked
      ```

## 2. Project Components

### esp-spoke-firmware
Primary firmware for the POV spoke display mounted to a bike wheel.

- README: [README-esp-spoke-firmware.md](README-esp-spoke-firmware.md)

### esp-bridge-firmware
Bridge firmware that receives host frames over USB serial and forwards them over BLE or ESP-NOW.

- README: [README-esp-bridge-firmware.md](README-esp-bridge-firmware.md)

### pov-proto
Shared wire protocol crate used by firmware and host tools.

- README: [README-pov-proto.md](README-pov-proto.md)

### pov-sender-cli
Host CLI for sending commands, images, and downloads through the bridge.

- README: [README-pov-sender-cli.md](README-pov-sender-cli.md)

## 3. Build Firmware Crates

From the repository root:

```sh
cargo build
```

This builds all ESP firmware crates in the workspace.

For specific targets:

```sh
cargo build -p esp-spoke-firmware --bin waveshare
cargo build -p esp-spoke-firmware --bin metro --no-default-features --features sk9822-strip
cargo build -p esp-bridge-firmware
```

See each component README for flashing and runtime details.

## 4. Build and Run Host CLI

The sender CLI lives outside the firmware workspace and uses its own toolchain/target configuration, see [README-pov-sender-cli.md](README-pov-sender-cli.md).