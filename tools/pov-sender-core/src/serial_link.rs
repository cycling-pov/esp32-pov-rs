use std::time::Duration;

use anyhow::Context;
use serialport::SerialPort;

pub(crate) fn open_serial_port(port_name: &str, baud: u32) -> anyhow::Result<Box<dyn SerialPort>> {
    serialport::new(port_name, baud)
        .timeout(Duration::from_secs(5))
        .open()
        .with_context(|| format!("Failed to open serial port {port_name}"))
}

pub fn list_serial_ports() -> anyhow::Result<Vec<String>> {
    let mut ports: Vec<String> = serialport::available_ports()
        .context("Failed to enumerate serial ports")?
        .into_iter()
        .map(|p| p.port_name)
        .collect();
    ports.sort();
    Ok(ports)
}
