# esp32-pov-rs
A persistence of vision embedded device for bike wheels

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

## 2. Build the project

From the repository root:

```sh
cargo build
```

During the build, the build script scans the `assets/` directory for PNG files and uses the first image it finds as the rendered source image.

If no image is found in `assets/`, the firmware falls back to a white display pattern.

To change the rendered image, replace the image file in `assets/` with your own PNG.

## 3. Flash and monitor

Connect the board over USB, then run:

```sh
cargo run
```

This will:

- build the firmware
- flash it using `espflash`
- open a serial monitor with `defmt` log decoding

If your board is not auto-detected, list ports and specify one manually:

```sh
espflash board-info
cargo run -- --port <serial-port>
```

## 4. Release build (optional)

For a smaller optimized image:

```sh
cargo run --release
```
