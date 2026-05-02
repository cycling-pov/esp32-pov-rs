# esp-spoke-firmware
A persistence of vision embedded device for bike wheels

## 1. Build the project

From the repository root:

```sh
cargo build -p esp-spoke-firmware
```

This builds the project defaults. Use the `--features` flag to enable certain features like espnow and the waveshare matrix.

If the "builtin-image" feature is enabled, the build script scans the `assets/` directory for PNG files and uses the first image it finds as the rendered source image.

If no image is found in `assets/`, the firmware falls back to a white display pattern.

To change the rendered image, replace the image file in `assets/` with your own PNG.

## 2. Flash and monitor

Connect the board over USB, then run:

```sh
cargo run -p esp-spoke-firmware
```

Use the following command to test the ADC monitor mode.
```sh
cargo run -p esp-spoke-firmware --bin=adc-test-software
```


This will:

- build the firmware
- flash it using `espflash`
- open a serial monitor with `defmt` log decoding

If your board is not auto-detected, list ports and specify one manually:

```sh
espflash board-info
cargo run -p esp-spoke-firmware -- --port <serial-port>
```

## 3. Release build (optional)

For a smaller optimized image:

```sh
cargo run -p esp-spoke-firmware --release
```
