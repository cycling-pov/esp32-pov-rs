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

## 2. esp-spoke-firmware
This is the primary firmware for the project, an LED persistence of light display mounted to a bike wheel.

Specific instructions for this project can be found [here](README-esp-spoke-firmware.md)

## 3. Build crates

From the repository root:

```sh
cargo build
```

This builds all esp crates in the workspace. See a subcrates' readme for specific build and run instructions.