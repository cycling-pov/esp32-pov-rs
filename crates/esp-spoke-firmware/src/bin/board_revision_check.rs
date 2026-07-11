use defmt::{info, warn};
use embassy_time::{Duration, Timer};

use esp_spoke_firmware::adc::{self, AdcDevice};

const BOARD_REV_MAX_RAW: u16 = 1750;

pub async fn check_board_revision() {
    let mut samples =
        adc::subscribe().expect("adc subscriber unavailable for board revision check");

    loop {
        adc::start_oneshot_mode(adc::AdcSelection::only(AdcDevice::BoardRev)).await;

        let sample = samples.next_message_pure().await;
        if sample.device != AdcDevice::BoardRev {
            continue;
        }

        info!("board revision sample raw={=u16}", sample.raw);
        if sample.raw <= BOARD_REV_MAX_RAW {
            return;
        }

        warn!(
            "board revision invalid raw={=u16} expected=<{}>",
            sample.raw, BOARD_REV_MAX_RAW
        );

        Timer::after(Duration::from_secs(1)).await;
    }
}
