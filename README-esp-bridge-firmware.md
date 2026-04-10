# esp-bridge-firmware
A wireless bridge firmware for ESP32-S3 that receives protocol frames over USB Serial/JTAG and forwards them over BLE Extended Advertising or ESP-NOW.

## 1. Build the project

From the repository root:

```sh
cargo build -p esp-bridge-firmware
```

This builds the bridge firmware binary at `crates/esp-bridge-firmware/src/bin/main.rs`.

## 2. Flash and monitor

Connect the bridge board over USB, then run:

```sh
cargo run -p esp-bridge-firmware
```

This will:

- build the firmware
- flash it using `espflash`
- open a serial monitor with `defmt` log decoding

If your board is not auto-detected, list ports and specify one manually:

```sh
espflash board-info
cargo run -p esp-bridge-firmware -- --port <serial-port>
```

After boot, the bridge:

- starts USB Serial/JTAG ingest
- starts BLE advertising backend
- starts ESP-NOW broadcast backend
- routes each incoming host frame based on the selected transport

**Ensure that the debug connection is not active before attempting to send commands** The UART is shared between the debug interface and the workstation for controlling the bridge.

## 3. Release build (optional)

For a smaller optimized image:

```sh
cargo run -p esp-bridge-firmware --release
```
