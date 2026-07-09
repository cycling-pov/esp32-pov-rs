# pov-sender-gui

Simple host GUI sender for POV bridge firmware.

## Build

```sh
cd tools/pov-sender-gui
cargo build
```

## Build (Web App)

This target runs in a Chromium browser and uses the Web Serial API.

Phone-browser caveat:
- Desktop Chromium is the supported path for the current bridge.
- Chrome on Android only exposes Web Serial for Bluetooth RFCOMM devices and does not yet support general wired USB serial for this app's transport.
- Safari/iOS does not support Web Serial.

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

## Deploy (Cloudflare Workers)

This project includes a Wrangler config and a minimal Worker that serves the
Trunk-generated static assets from `dist`.

Prerequisites:
- Node.js 18+
- Cloudflare account

One-time setup:

```sh
cd tools/pov-sender-gui
npm install
npx wrangler login
```

Deploy:

```sh
cd tools/pov-sender-gui
npm run deploy
```

Equivalent manual steps:

```sh
cd tools/pov-sender-gui
trunk build --release
npx wrangler deploy
```

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

If testing on a phone, expect the current build to be informational only unless the bridge is exposed as a Bluetooth RFCOMM serial device.

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
