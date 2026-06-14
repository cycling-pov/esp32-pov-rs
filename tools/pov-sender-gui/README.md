# pov-sender-gui

Simple host GUI sender for POV bridge firmware.

## Build

```sh
cd tools/pov-sender-gui
cargo build
```

## Run

```sh
cd tools/pov-sender-gui
cargo run
```

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
