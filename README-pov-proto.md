# pov-proto
Shared wire protocol crate for the POV project. It defines frame formats, transfer chunking/assembly, command packets, and image wire encoding/decoding helpers.

## 1. Build the crate

From the repository root:

```sh
cargo build -p pov-proto
```

This builds the base crate without optional image encode/decode helpers.

To build with image encoding support:

```sh
cargo build -p pov-proto --features image-encode
```

To build with image decoding support:

```sh
cargo build -p pov-proto --features image-decode
```

To build with all optional helpers enabled:

```sh
cargo build -p pov-proto --all-features
```

## 2. What this crate provides

- transfer packet encoding/decoding (`Packet`, `DownloadChunk`, `CommandFrame`)
- chunk iterator and transfer assembly (`ChunkIter`, `TransferAssembly`)
- download type tagging (`DownloadKind`: display image, OTA image, video)
- bridge envelope types used by host-to-bridge serial link (`BridgeFrame`)
- image wire helpers for RGB-to-wire and wire-to-RGB paths (feature-gated)
