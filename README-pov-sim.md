# pov-sim
Workstation simulation tool to preview images and test algorithms on a workstation computer before implementation in firmware.

## 1. Build the tool

Builds must be executed from the tool's directory.

```sh
cd tools/pov-sim
cargo build
```

The tool contains its own `rust-toolchain.toml` and `.cargo/config.toml` so it builds with stable Rust and uses the host target on Windows, macOS, or Linux.

## 2. Run

From `tools/pov-sim`:

```sh
cargo run
```

## 3. Keyboard Commands

The simulation provides several keyboard commands for modifying the state of the simulation.
* `A` advances to the next image selection
* `F` toggles the frame count
* `G` toggles the frame graph
* `T` toggles the theme (light/dark)
* `Up`/`Down` changes the current rotation rate
* `Left`/`Right` changes the light fade duration
