# pov-sender-gui

Simple host GUI sender for POV bridge firmware.

## Build

```sh
cd tools/pov-sender-gui
cargo build
```

## Build (Web App)

This target runs in a Chromium browser and uses the Web Serial API.

Prerequisites:
- Rust target: `wasm32-unknown-unknown`
- Trunk: `cargo install trunk`
- Browser with Web Serial support (Chromium/Edge)

Build and serve locally:

```sh
cd tools/pov-sender-gui
rustup target add wasm32-unknown-unknown
trunk serve --release
```

Build static assets for deployment:

```sh
cd tools/pov-sender-gui
trunk build --release
```

Generated web assets are emitted under `tools/pov-sender-gui/dist`.

## Run

```sh
cd tools/pov-sender-gui
cargo run
```

## Run (Web App)

Open the URL printed by `trunk serve`, then:
- Click `Connect` to grant serial-device access.
- Select the granted port in the port dropdown.
- Use command tabs that do not require local file access.

## Features

- Manual serial port refresh panel
- Serial port selection, transport, baud, and repeat controls
- Buttons for all sender actions:
  - Send image
  - Send download
  - Display off
  - Next image
  - Randomize display
- File picker for image and download payload

Web build status:
- Implemented: Web Serial connection, command send path, browser file upload for Send Image/Send OTA
- Not implemented yet: storage-stats response reads
