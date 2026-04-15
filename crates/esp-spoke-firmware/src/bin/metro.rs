use defmt::info;

#[cfg(not(feature = "sk9822-strip"))]
pub fn initialize_metro_output() {
    info!("Adafruit Metro ESP32-S3 target active");
}

