# esp-spoke-firmware
A persistence of vision embedded device for bike wheels

## 1. Build the project

From the repository root:

```sh
cargo build -p esp-spoke-firmware --bin waveshare
```

This builds all crates in the workspace, which contains several firmware projects.

```sh
cargo build -p esp-spoke-firmware --bin waveshare
```

This builds the Waveshare target binary with the Waveshare Matrix component enabled.

To build for Adafruit Metro ESP32-S3 (without Waveshare Matrix output):

```sh
cargo build -p esp-spoke-firmware --bin metro --no-default-features
```

To build Metro with SK9822 strip output enabled:

```sh
cargo build -p esp-spoke-firmware --bin metro --no-default-features --features sk9822-strip
```

During the build, the build script scans the `assets/` directory for PNG files and uses the first image it finds as the rendered source image.

If no image is found in `assets/`, the firmware falls back to a white display pattern.

To change the rendered image, replace the image file in `assets/` with your own PNG.

## 2. Flash and monitor

Connect the board over USB, then run:

```sh
cargo run -p esp-spoke-firmware --bin waveshare
```

This will:

- build the firmware
- flash it using `espflash`
- open a serial monitor with `defmt` log decoding

If your board is not auto-detected, list ports and specify one manually:

```sh
espflash board-info
cargo run -p esp-spoke-firmware --bin waveshare -- --port <serial-port>
```

To run the Metro target:

```sh
cargo run -p esp-spoke-firmware --bin metro --no-default-features
```

To run Metro with SK9822 strip output:

```sh
cargo run -p esp-spoke-firmware --bin metro --no-default-features --features sk9822-strip
```

Both board targets use the same `src/bin/main.rs` entry file. Board-specific logic lives in separate files under `src/bin/`.

## 3. Release build (optional)

For a smaller optimized image:

```sh
cargo run -p esp-spoke-firmware --release
```
