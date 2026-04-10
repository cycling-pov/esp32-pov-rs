use core::cell::RefCell;

use bt_hci::{
    controller::ExternalController,
    param::{LeAdvReportsIter, LeExtAdvDataStatus, LeExtAdvReport, LeExtAdvReportsIter},
};
use defmt::{debug, info, warn};
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Timer};
use esp_radio::ble::controller::BleConnector;
use trouble_host::prelude::{
    Address, DefaultPacketPool, EventHandler, Host, HostResources, PhySet, ScanConfig, Scanner,
};

use pov_proto::transfer::ParseError;

pub const MANUFACTURER_DATA_AD_TYPE: u8 = 0xFF;
pub const EXPECTED_MANUFACTURER_COMPANY_ID: u16 = 0xFFFF;

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 1;
const MAX_EXTENDED_AD_PAYLOAD_BYTES: usize = 512;

pub type BleController = ExternalController<BleConnector<'static>, 1>;

#[derive(Debug, defmt::Format)]
pub enum BleAdvertisementError {
    MalformedAdStructure,
    ManufacturerDataTooShort,
    Ingest(ParseError),
}

impl From<ParseError> for BleAdvertisementError {
    fn from(value: ParseError) -> Self {
        Self::Ingest(value)
    }
}

struct ExtendedAdvertisementAssembly {
    active: bool,
    source_addr: [u8; 6],
    adv_sid: u8,
    legacy: bool,
    len: usize,
    bytes: [u8; MAX_EXTENDED_AD_PAYLOAD_BYTES],
}

impl ExtendedAdvertisementAssembly {
    const fn new() -> Self {
        Self {
            active: false,
            source_addr: [0; 6],
            adv_sid: 0,
            legacy: false,
            len: 0,
            bytes: [0; MAX_EXTENDED_AD_PAYLOAD_BYTES],
        }
    }

    fn reset(&mut self) {
        self.active = false;
        self.len = 0;
    }

    fn matches_source(&self, source_addr: [u8; 6], adv_sid: u8, legacy: bool) -> bool {
        self.source_addr == source_addr && self.adv_sid == adv_sid && self.legacy == legacy
    }

    fn start_source(&mut self, source_addr: [u8; 6], adv_sid: u8, legacy: bool) {
        self.active = true;
        self.source_addr = source_addr;
        self.adv_sid = adv_sid;
        self.legacy = legacy;
        self.len = 0;
    }

    fn append(&mut self, data: &[u8]) -> bool {
        if self.len + data.len() > self.bytes.len() {
            return false;
        }

        let end = self.len + data.len();
        self.bytes[self.len..end].copy_from_slice(data);
        self.len = end;
        true
    }
}

struct BleScanEventHandler {
    extended_assembly: RefCell<ExtendedAdvertisementAssembly>,
}

impl BleScanEventHandler {
    fn new() -> Self {
        Self {
            extended_assembly: RefCell::new(ExtendedAdvertisementAssembly::new()),
        }
    }

    fn ingest_extended_report_payload(&self, payload: &[u8], report: &LeExtAdvReport<'_>) {
        let result = if report.event_kind.legacy() {
            ingest_legacy_advertisement(payload)
        } else {
            ingest_extended_advertisement(payload)
        };

        match result {
            Ok(true) => info!(
                "BLE extended advertisement handled: bytes={=usize} rssi={=i8} legacy={=bool}",
                payload.len(),
                report.rssi,
                report.event_kind.legacy()
            ),
            Ok(false) => {}
            Err(err) => warn!("BLE extended advertisement handling failed: {:?}", err),
        }
    }
}

impl EventHandler for BleScanEventHandler {
    // This was implemented from an initial build and kept since we have the code. It can be used for simple commands that don't require a lot of data.
    fn on_adv_reports(&self, reports: LeAdvReportsIter) {
        for report in reports {
            let report = match report {
                Ok(report) => report,
                Err(_) => {
                    warn!("BLE legacy advertisement report parsing failed");
                    continue;
                }
            };

            match ingest_legacy_advertisement(report.data) {
                Ok(true) => info!(
                    "BLE legacy advertisement handled: bytes={=usize} rssi={=i8}",
                    report.data.len(),
                    report.rssi
                ),
                Ok(false) => {}
                Err(err) => warn!("BLE legacy advertisement handling failed: {:?}", err),
            }
        }
    }

    fn on_ext_adv_reports(&self, reports: LeExtAdvReportsIter) {
        for report in reports {
            let report = match report {
                Ok(report) => report,
                Err(_) => {
                    warn!("BLE extended advertisement report parsing failed");
                    continue;
                }
            };

            let source_addr = report.addr.into_inner();
            let adv_sid = report.adv_sid;
            let legacy = report.event_kind.legacy();
            let status = report.event_kind.data_status();

            match status {
                LeExtAdvDataStatus::Complete => {
                    let mut assembled_payload = [0u8; MAX_EXTENDED_AD_PAYLOAD_BYTES];
                    let assembled_len = {
                        let mut assembly = self.extended_assembly.borrow_mut();

                        if assembly.active {
                            if !assembly.matches_source(source_addr, adv_sid, legacy) {
                                assembly.reset();
                                assembly.start_source(source_addr, adv_sid, legacy);
                            }

                            if !assembly.append(report.data) {
                                warn!(
                                    "BLE extended advertisement assembly overflow: accumulated={=usize} incoming={=usize}",
                                    assembly.len,
                                    report.data.len()
                                );
                                assembly.reset();
                                continue;
                            }

                            let len = assembly.len;
                            assembled_payload[..len].copy_from_slice(&assembly.bytes[..len]);
                            assembly.reset();
                            len
                        } else {
                            if report.data.len() > assembled_payload.len() {
                                warn!(
                                    "BLE extended advertisement payload too large: bytes={=usize}",
                                    report.data.len()
                                );
                                continue;
                            }

                            let len = report.data.len();
                            assembled_payload[..len].copy_from_slice(report.data);
                            len
                        }
                    };

                    self.ingest_extended_report_payload(
                        &assembled_payload[..assembled_len],
                        &report,
                    );
                }
                LeExtAdvDataStatus::IncompleteMoreExpected => {
                    let mut assembly = self.extended_assembly.borrow_mut();
                    if !assembly.active || !assembly.matches_source(source_addr, adv_sid, legacy) {
                        assembly.start_source(source_addr, adv_sid, legacy);
                    }

                    if !assembly.append(report.data) {
                        warn!(
                            "BLE extended advertisement assembly overflow: accumulated={=usize} incoming={=usize}",
                            assembly.len,
                            report.data.len()
                        );
                        assembly.reset();
                    }
                }
                LeExtAdvDataStatus::IncompleteTruncated => {
                    self.extended_assembly.borrow_mut().reset();
                    debug!("BLE extended advertisement truncated");
                }
                LeExtAdvDataStatus::Reserved => {
                    self.extended_assembly.borrow_mut().reset();
                    warn!("BLE extended advertisement reported reserved data status");
                }
            }
        }
    }
}

pub fn start_ble_backend(spawner: Spawner, controller: BleController) {
    if spawner.spawn(ble_backend_task(controller)).is_err() {
        info!("BLE backend task already running or unavailable");
    }
}

#[embassy_executor::task]
pub async fn ble_backend_task(controller: BleController) {
    let address = Address::random([0xff, 0x8f, 0x1b, 0x05, 0xe4, 0xff]);
    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources).set_random_address(address);
    let Host {
        central,
        mut runner,
        ..
    } = stack.build();
    let handler = BleScanEventHandler::new();
    let mut scanner = Scanner::new(central);
    // Balance BLE advertisement reassembly with ESP-NOW coexistence.
    // interval=1280ms, window=640ms gives ~50% duty cycle, allowing both BLE downloads
    // and ESP-NOW to work reliably without starving either.
    let config = ScanConfig {
        active: true,
        phys: PhySet::M1,
        interval: Duration::from_millis(1280),
        window: Duration::from_millis(640),
        timeout: Duration::from_secs(0),
        ..ScanConfig::default()
    };

    info!(
        "BLE broadcast receiver backend starting: active={=bool} interval_ms={=u64} window_ms={=u64}",
        config.active,
        config.interval.as_millis(),
        config.window.as_millis()
    );

    let result = select(runner.run_with_handler(&handler), async {
        let _session = match scanner.scan_ext(&config).await {
            Ok(session) => session,
            Err(err) => return Err(err),
        };

        info!("BLE extended scan started");

        core::future::pending::<()>().await;
        #[allow(unreachable_code)]
        Ok(())
    })
    .await;

    match result {
        Either::First(Err(_)) => warn!("BLE host runner exited with error"),
        Either::First(Ok(())) => warn!("BLE host runner exited unexpectedly"),
        Either::Second(Err(_)) => warn!("BLE scanner exited with error"),
        Either::Second(Ok(())) => warn!("BLE scanner exited unexpectedly"),
    }

    loop {
        Timer::after(Duration::from_secs(5)).await;
    }
}

pub fn ingest_manufacturer_data(payload: &[u8]) -> Result<(), ParseError> {
    super::ingest_manufacturer_data(payload)
}

pub fn ingest_legacy_advertisement(
    advertisement_data: &[u8],
) -> Result<bool, BleAdvertisementError> {
    ingest_advertisement_data(advertisement_data)
}

pub fn ingest_extended_advertisement(
    advertisement_data: &[u8],
) -> Result<bool, BleAdvertisementError> {
    ingest_advertisement_data(advertisement_data)
}

pub fn ingest_advertisement_data(advertisement_data: &[u8]) -> Result<bool, BleAdvertisementError> {
    let mut offset = 0usize;

    while offset < advertisement_data.len() {
        let structure_len = advertisement_data[offset] as usize;
        offset += 1;

        if structure_len == 0 {
            break;
        }

        let structure_end = offset + structure_len;
        if structure_end > advertisement_data.len() {
            return Ok(false);
        }

        let ad_type = advertisement_data[offset];
        let ad_payload = &advertisement_data[offset + 1..structure_end];

        if ad_type == MANUFACTURER_DATA_AD_TYPE {
            if ad_payload.len() < 2 {
                return Err(BleAdvertisementError::ManufacturerDataTooShort);
            }

            let company_id = u16::from_le_bytes([ad_payload[0], ad_payload[1]]);
            if company_id != EXPECTED_MANUFACTURER_COMPANY_ID {
                offset = structure_end;
                continue;
            }

            let manufacturer_payload = &ad_payload[2..];
            if manufacturer_payload.is_empty() {
                warn!("BLE manufacturer payload is empty");
                return Err(BleAdvertisementError::ManufacturerDataTooShort);
            }

            if let Err(err) = ingest_manufacturer_data(manufacturer_payload) {
                warn!("BLE manufacturer payload processing failed: {:?}", err);
                return Err(err.into());
            }
            return Ok(true);
        }

        offset = structure_end;
    }
    Ok(false)
}
