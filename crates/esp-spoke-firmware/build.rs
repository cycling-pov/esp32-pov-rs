use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const GENERATED_BITMAP_WIDTH: u32 = 64;
const GENERATED_BITMAP_HEIGHT: u32 = 64;

fn main() {
    generate_asset_bitmap();
    copy_partition_table();
    linker_be_nice();
    println!("cargo:rustc-link-arg=-Tdefmt.x");
    // make sure linkall.x is the last linker script (otherwise might cause problems with flip-link)
    println!("cargo:rustc-link-arg=-Tlinkall.x");
}

/// Copy the correct partition-table CSV into `target/` based on the active
/// flash-size feature.  The runner in `.cargo/config.toml` always points at
/// `target/esp-spoke-firmware-partitions.csv`, so this keeps it up-to-date
/// automatically whenever the active feature changes.
fn copy_partition_table() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR");
    let manifest_path = PathBuf::from(&manifest_dir);

    let csv_name = if cfg!(feature = "flash-16mb") {
        "partitions-16mb.csv"
    } else {
        // default: flash-4mb
        "partitions-4mb.csv"
    };
    let src = manifest_path.join(csv_name);

    // Write to `<workspace_root>/target/esp-spoke-firmware-partitions.csv`.
    // CARGO_MANIFEST_DIR is `crates/esp-spoke-firmware`, so `../..` reaches the workspace root.
    let dest = manifest_path
        .join("../..")
        .join("target")
        .join("esp-spoke-firmware-partitions.csv");

    std::fs::copy(&src, &dest)
        .unwrap_or_else(|e| panic!("failed to copy {csv_name} to target/: {e}"));

    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_FLASH_16MB");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_FLASH_4MB");
}

fn generate_asset_bitmap() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("missing manifest dir");
    let assets_dir = Path::new(&manifest_dir).join("assets");
    let out_dir = std::env::var("OUT_DIR").expect("missing OUT_DIR");
    let output_path = Path::new(&out_dir).join("asset_bitmap.rs");

    println!("cargo:rerun-if-changed={}", assets_dir.display());

    let image_paths = asset_png_paths(&assets_dir);
    let generated = if let Some(image_path) = image_paths.first() {
        println!("cargo:rerun-if-changed={}", image_path.display());
        generate_bitmap_source_from_png(image_path)
    } else {
        generate_off_bitmap_source()
    };

    std::fs::write(&output_path, generated)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", output_path.display()));
}

fn asset_png_paths(assets_dir: &Path) -> Vec<PathBuf> {
    let mut image_paths = Vec::new();

    let Ok(entries) = std::fs::read_dir(assets_dir) else {
        return image_paths;
    };

    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("failed to read asset entry: {error}"));
        let path = entry.path();

        let is_png = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("png"));

        if is_png {
            image_paths.push(path);
        }
    }

    image_paths.sort();
    image_paths
}

fn generate_bitmap_source_from_png(png_path: &Path) -> String {
    let image = image::ImageReader::open(png_path)
        .unwrap_or_else(|error| panic!("failed to open {}: {error}", png_path.display()))
        .decode()
        .unwrap_or_else(|error| panic!("failed to decode {}: {error}", png_path.display()))
        .resize_exact(
            GENERATED_BITMAP_WIDTH,
            GENERATED_BITMAP_HEIGHT,
            image::imageops::FilterType::Triangle,
        )
        .to_rgba8();

    let (width, height) = image.dimensions();
    let mut generated = String::new();

    writeln!(
        &mut generated,
        "pub const GENERATED_BITMAP_METADATA: BitmapStorageMetadata = BitmapStorageMetadata {{ width: {}, height: {} }};",
        width,
        height
    )
    .expect("failed to write generated metadata");
    writeln!(
        &mut generated,
        "pub const GENERATED_BITMAP_PIXEL_COUNT: usize = GENERATED_BITMAP_METADATA.pixel_count();"
    )
    .expect("failed to write generated pixel count");
    writeln!(
        &mut generated,
        "pub static GENERATED_BITMAP: [RGB8; GENERATED_BITMAP_PIXEL_COUNT] = ["
    )
    .expect("failed to start generated pixel array");

    for pixel in image.pixels() {
        let [red, green, blue, alpha] = pixel.0;
        let (red, green, blue) = if alpha == 0 {
            (0, 0, 0)
        } else {
            (red, green, blue)
        };

        writeln!(
            &mut generated,
            "    RGB8 {{ r: {red}, g: {green}, b: {blue} }},"
        )
        .expect("failed to write generated pixel");
    }

    writeln!(&mut generated, "];").expect("failed to finish generated pixel array");

    generated
}

fn generate_off_bitmap_source() -> String {
    let mut generated = String::new();

    writeln!(
        &mut generated,
        "pub const GENERATED_BITMAP_METADATA: BitmapStorageMetadata = BitmapStorageMetadata {{ width: {}, height: {} }};",
        GENERATED_BITMAP_WIDTH,
        GENERATED_BITMAP_HEIGHT
    )
    .expect("failed to write fallback metadata");
    writeln!(
        &mut generated,
        "pub const GENERATED_BITMAP_PIXEL_COUNT: usize = GENERATED_BITMAP_METADATA.pixel_count();"
    )
    .expect("failed to write fallback pixel count");
    writeln!(
        &mut generated,
        "pub static GENERATED_BITMAP: [RGB8; GENERATED_BITMAP_PIXEL_COUNT] = [RGB8 {{ r: 64, g: 64, b: 64 }}; GENERATED_BITMAP_PIXEL_COUNT];"
    )
    .expect("failed to write fallback pixel array");

    generated
}

fn linker_be_nice() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        let kind = &args[1];
        let what = &args[2];

        match kind.as_str() {
            "undefined-symbol" => match what.as_str() {
                what if what.starts_with("_defmt_") => {
                    eprintln!();
                    eprintln!(
                        "💡 `defmt` not found - make sure `defmt.x` is added as a linker script and you have included `use defmt_rtt as _;`"
                    );
                    eprintln!();
                }
                "_stack_start" => {
                    eprintln!();
                    eprintln!("💡 Is the linker script `linkall.x` missing?");
                    eprintln!();
                }
                what if what.starts_with("esp_rtos_") => {
                    eprintln!();
                    eprintln!(
                        "💡 `esp-radio` has no scheduler enabled. Make sure you have initialized `esp-rtos` or provided an external scheduler."
                    );
                    eprintln!();
                }
                "embedded_test_linker_file_not_added_to_rustflags" => {
                    eprintln!();
                    eprintln!(
                        "💡 `embedded-test` not found - make sure `embedded-test.x` is added as a linker script for tests"
                    );
                    eprintln!();
                }
                "free"
                | "malloc"
                | "calloc"
                | "get_free_internal_heap_size"
                | "malloc_internal"
                | "realloc_internal"
                | "calloc_internal"
                | "free_internal" => {
                    eprintln!();
                    eprintln!(
                        "💡 Did you forget the `esp-alloc` dependency or didn't enable the `compat` feature on it?"
                    );
                    eprintln!();
                }
                _ => (),
            },
            // we don't have anything helpful for "missing-lib" yet
            _ => {
                std::process::exit(1);
            }
        }

        std::process::exit(0);
    }

    println!(
        "cargo:rustc-link-arg=-Wl,--error-handling-script={}",
        std::env::current_exe().unwrap().display()
    );
}
