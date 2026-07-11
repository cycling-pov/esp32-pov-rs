use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::pubsub::{PubSubChannel, Subscriber};
use embassy_time::{Duration, Timer};
use esp_hal::Blocking;
use esp_hal::analog::adc::{
    Adc, AdcCalScheme, AdcChannel, AdcConfig, AdcPin, Attenuation, RegisterAccess,
};
use esp_hal::peripherals::{ADC1, GPIO2, GPIO4, GPIO5, GPIO8};

use crate::storage;

const DEFAULT_RATE_HZ: u16 = 20;
const MAX_RATE_HZ: u16 = 2_000;
const ADC_SAMPLE_CAPACITY: usize = 32;
const ADC_SAMPLE_SUBSCRIBERS: usize = 4;
const ADC_SAMPLE_PUBLISHERS: usize = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, defmt::Format)]
pub enum AdcDevice {
    BoardRev,
    HallEffectSensor2,
    BatteryVoltage,
    HallEffectSensor1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, defmt::Format)]
pub enum AdcSampleSource {
    Monitor,
    Oneshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, defmt::Format)]
pub struct AdcSelection {
    pub board_rev: Option<bool>,
    pub hall_effect_sensor2: Option<bool>,
    pub battery_voltage: Option<bool>,
    pub hall_effect_sensor1: Option<bool>,
}

impl AdcSelection {
    pub const fn none() -> Self {
        Self {
            board_rev: None,
            hall_effect_sensor2: None,
            battery_voltage: None,
            hall_effect_sensor1: None,
        }
    }

    pub const fn all() -> Self {
        Self {
            board_rev: Some(true),
            hall_effect_sensor2: Some(true),
            battery_voltage: Some(true),
            hall_effect_sensor1: Some(true),
        }
    }

    pub const fn only(device: AdcDevice) -> Self {
        match device {
            AdcDevice::BoardRev => Self {
                board_rev: Some(true),
                ..Self::none()
            },
            AdcDevice::HallEffectSensor2 => Self {
                hall_effect_sensor2: Some(true),
                ..Self::none()
            },
            AdcDevice::BatteryVoltage => Self {
                battery_voltage: Some(true),
                ..Self::none()
            },
            AdcDevice::HallEffectSensor1 => Self {
                hall_effect_sensor1: Some(true),
                ..Self::none()
            },
        }
    }

    pub const fn disabled(device: AdcDevice) -> Self {
        match device {
            AdcDevice::BoardRev => Self {
                board_rev: Some(false),
                ..Self::none()
            },
            AdcDevice::HallEffectSensor2 => Self {
                hall_effect_sensor2: Some(false),
                ..Self::none()
            },
            AdcDevice::BatteryVoltage => Self {
                battery_voltage: Some(false),
                ..Self::none()
            },
            AdcDevice::HallEffectSensor1 => Self {
                hall_effect_sensor1: Some(false),
                ..Self::none()
            },
        }
    }

    pub const fn stop_all() -> Self {
        Self {
            board_rev: Some(false),
            hall_effect_sensor2: Some(false),
            battery_voltage: Some(false),
            hall_effect_sensor1: Some(false),
        }
    }

    pub const fn is_enabled(value: Option<bool>) -> bool {
        matches!(value, Some(true))
    }

    pub fn apply(&mut self, update: Self) {
        if let Some(value) = update.board_rev {
            self.board_rev = Some(value);
        }
        if let Some(value) = update.hall_effect_sensor2 {
            self.hall_effect_sensor2 = Some(value);
        }
        if let Some(value) = update.battery_voltage {
            self.battery_voltage = Some(value);
        }
        if let Some(value) = update.hall_effect_sensor1 {
            self.hall_effect_sensor1 = Some(value);
        }
    }

    pub const fn any(self) -> bool {
        Self::is_enabled(self.board_rev)
            || Self::is_enabled(self.hall_effect_sensor2)
            || Self::is_enabled(self.battery_voltage)
            || Self::is_enabled(self.hall_effect_sensor1)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, defmt::Format)]
pub struct AdcSample {
    pub device: AdcDevice,
    pub raw: u16,
    pub source: AdcSampleSource,
}

#[derive(Clone, Copy)]
enum AdcCommand {
    StartMonitor { selection: AdcSelection },
    StopMonitor,
    RequestOneshot { selection: AdcSelection },
    SetMonitorSampleRateHz { hz: u16 },
}

pub type AdcSampleSubscriber = Subscriber<
    'static,
    CriticalSectionRawMutex,
    AdcSample,
    ADC_SAMPLE_CAPACITY,
    ADC_SAMPLE_SUBSCRIBERS,
    ADC_SAMPLE_PUBLISHERS,
>;

static ADC_COMMAND_CHANNEL: Channel<CriticalSectionRawMutex, AdcCommand, 8> = Channel::new();
static ADC_SAMPLES: PubSubChannel<
    CriticalSectionRawMutex,
    AdcSample,
    ADC_SAMPLE_CAPACITY,
    ADC_SAMPLE_SUBSCRIBERS,
    ADC_SAMPLE_PUBLISHERS,
> = PubSubChannel::new();

pub fn init(
    spawner: Spawner,
    adc1: ADC1<'static>,
    gpio2: GPIO2<'static>,
    gpio4: GPIO4<'static>,
    gpio5: GPIO5<'static>,
    gpio8: GPIO8<'static>,
) {
    spawner
        .spawn(adc_task(adc1, gpio2, gpio4, gpio5, gpio8).expect("failed to build adc task token"));
}

pub fn subscribe() -> Option<AdcSampleSubscriber> {
    ADC_SAMPLES.subscriber().ok()
}

pub async fn start_monitor_mode(selection: AdcSelection) {
    ADC_COMMAND_CHANNEL
        .send(AdcCommand::StartMonitor { selection })
        .await;
}

pub async fn stop_monitor_mode() {
    ADC_COMMAND_CHANNEL.send(AdcCommand::StopMonitor).await;
}

pub async fn start_oneshot_mode(selection: AdcSelection) {
    ADC_COMMAND_CHANNEL
        .send(AdcCommand::RequestOneshot { selection })
        .await;
}

pub async fn set_monitor_sample_rate_hz(hz: u16) {
    ADC_COMMAND_CHANNEL
        .send(AdcCommand::SetMonitorSampleRateHz { hz })
        .await;
}

fn clamp_rate_hz(hz: u16) -> u16 {
    hz.clamp(1, MAX_RATE_HZ)
}

fn monitor_interval(rate_hz: u16) -> Duration {
    Duration::from_hz(rate_hz as u64)
}

async fn read_adc_sample<'a, 'd, ADCX, PIN, CS>(
    adc: &'a mut Adc<'d, ADCX, Blocking>,
    pin: &'a mut AdcPin<PIN, ADCX, CS>,
) -> u16
where
    ADCX: RegisterAccess + 'd,
    PIN: AdcChannel,
    CS: AdcCalScheme<ADCX>,
{
    loop {
        if let Ok(sample) = adc.read_oneshot(pin) {
            break sample;
        }
        Timer::after(Duration::from_millis(1)).await;
    }
}

fn publish_sample(sample: AdcSample) {
    ADC_SAMPLES.immediate_publisher().publish_immediate(sample);
}

async fn sample_selected(
    adc: &mut Adc<'static, ADC1<'static>, Blocking>,
    gpio2: &mut AdcPin<GPIO2<'static>, ADC1<'static>>,
    gpio4: &mut AdcPin<GPIO4<'static>, ADC1<'static>>,
    gpio5: &mut AdcPin<GPIO5<'static>, ADC1<'static>>,
    gpio8: &mut AdcPin<GPIO8<'static>, ADC1<'static>>,
    selection: AdcSelection,
    source: AdcSampleSource,
) {
    if AdcSelection::is_enabled(selection.board_rev) {
        let raw = read_adc_sample(adc, gpio2).await;
        publish_sample(AdcSample {
            device: AdcDevice::BoardRev,
            raw,
            source,
        });
    }
    if AdcSelection::is_enabled(selection.hall_effect_sensor2) {
        let raw = read_adc_sample(adc, gpio4).await;
        publish_sample(AdcSample {
            device: AdcDevice::HallEffectSensor2,
            raw,
            source,
        });
    }
    if AdcSelection::is_enabled(selection.battery_voltage) {
        let raw = read_adc_sample(adc, gpio5).await;
        publish_sample(AdcSample {
            device: AdcDevice::BatteryVoltage,
            raw,
            source,
        });
    }
    if AdcSelection::is_enabled(selection.hall_effect_sensor1) {
        let raw = read_adc_sample(adc, gpio8).await;
        publish_sample(AdcSample {
            device: AdcDevice::HallEffectSensor1,
            raw,
            source,
        });
    }
}

#[embassy_executor::task]
pub async fn adc_task(
    adc1: ADC1<'static>,
    gpio2_raw: GPIO2<'static>,
    gpio4_raw: GPIO4<'static>,
    gpio5_raw: GPIO5<'static>,
    gpio8_raw: GPIO8<'static>,
) -> ! {
    let mut rate_hz = storage::get_adc_monitor_sample_rate_hz().await;
    if rate_hz == 0 {
        rate_hz = DEFAULT_RATE_HZ;
    }
    rate_hz = clamp_rate_hz(rate_hz);

    let mut config: AdcConfig<_> = AdcConfig::new();

    // BOARD_REV is < 0.05V, so no attenuation is needed
    let mut gpio2 = config.enable_pin(gpio2_raw, Attenuation::_0dB);
    // SENSOR 1 and 2 are scaled to a max of 2.6V
    let mut gpio8 = config.enable_pin(gpio8_raw, Attenuation::_11dB);
    let mut gpio4 = config.enable_pin(gpio4_raw, Attenuation::_11dB);
    // BATT_VOLTAGE is scaled to a max of 2.5V
    let mut gpio5 = config.enable_pin(gpio5_raw, Attenuation::_11dB);

    let mut adc = Adc::new(adc1, config);

    let mut monitor_selection = AdcSelection::none();
    info!("adc:task started sample_rate_hz={=u16}", rate_hz);

    loop {
        if monitor_selection.any() {
            match select(
                ADC_COMMAND_CHANNEL.receive(),
                Timer::after(monitor_interval(rate_hz)),
            )
            .await
            {
                Either::First(cmd) => match cmd {
                    AdcCommand::StartMonitor { selection } => {
                        monitor_selection.apply(selection);
                        info!("adc:monitor start");
                    }
                    AdcCommand::StopMonitor => {
                        monitor_selection = AdcSelection::stop_all();
                        info!("adc:monitor stop");
                    }
                    AdcCommand::RequestOneshot { selection } => {
                        sample_selected(
                            &mut adc,
                            &mut gpio2,
                            &mut gpio4,
                            &mut gpio5,
                            &mut gpio8,
                            selection,
                            AdcSampleSource::Oneshot,
                        )
                        .await;
                    }
                    AdcCommand::SetMonitorSampleRateHz { hz } => {
                        rate_hz = clamp_rate_hz(hz);
                        if storage::set_adc_monitor_sample_rate_hz(rate_hz)
                            .await
                            .is_err()
                        {
                            warn!("adc:set sample rate persist failed");
                        }
                    }
                },
                Either::Second(_) => {
                    sample_selected(
                        &mut adc,
                        &mut gpio2,
                        &mut gpio4,
                        &mut gpio5,
                        &mut gpio8,
                        monitor_selection,
                        AdcSampleSource::Monitor,
                    )
                    .await;
                }
            }
        } else {
            match ADC_COMMAND_CHANNEL.receive().await {
                AdcCommand::StartMonitor { selection } => {
                    monitor_selection.apply(selection);
                    info!("adc:monitor start");
                }
                AdcCommand::StopMonitor => {
                    monitor_selection = AdcSelection::stop_all();
                }
                AdcCommand::RequestOneshot { selection } => {
                    sample_selected(
                        &mut adc,
                        &mut gpio2,
                        &mut gpio4,
                        &mut gpio5,
                        &mut gpio8,
                        selection,
                        AdcSampleSource::Oneshot,
                    )
                    .await;
                }
                AdcCommand::SetMonitorSampleRateHz { hz } => {
                    rate_hz = clamp_rate_hz(hz);
                    if storage::set_adc_monitor_sample_rate_hz(rate_hz)
                        .await
                        .is_err()
                    {
                        warn!("adc:set sample rate persist failed");
                    }
                }
            }
        }
    }
}
