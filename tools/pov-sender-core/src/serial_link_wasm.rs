pub fn list_serial_ports() -> anyhow::Result<Vec<String>> {
    anyhow::bail!("Serial port enumeration is not available in wasm sender core")
}
