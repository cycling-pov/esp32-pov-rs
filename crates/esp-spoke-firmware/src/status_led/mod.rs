use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Timer};
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::peripherals::GPIO46;

const STATUS_LED_BLINK_SLOW: Duration = Duration::from_millis(600);
const STATUS_LED_BLINK_FAST: Duration = Duration::from_millis(180);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatusLedRequest {
    /// `None` keeps the LED on solid. `Some(period)` toggles on that cadence.
    pub blink_period: Option<Duration>,
}

impl StatusLedRequest {
    pub const OFF: Self = Self {
        blink_period: Some(Duration::from_millis(0)),
    };

    pub const SOLID_ON: Self = Self { blink_period: None };

    pub const BLINK_SLOW: Self = Self {
        blink_period: Some(STATUS_LED_BLINK_SLOW),
    };

    pub const BLINK_FAST: Self = Self {
        blink_period: Some(STATUS_LED_BLINK_FAST),
    };
}

static STATUS_LED_REQUEST_CHANNEL: Channel<CriticalSectionRawMutex, StatusLedRequest, 8> =
    Channel::new();

pub fn try_send_request(request: StatusLedRequest) -> bool {
    if STATUS_LED_REQUEST_CHANNEL.try_send(request).is_ok() {
        true
    } else {
        warn!("status_led:request dropped channel full");
        false
    }
}

pub async fn send_request(request: StatusLedRequest) {
    STATUS_LED_REQUEST_CHANNEL.send(request).await;
}

pub fn init(pin: GPIO46<'static>, spawner: Spawner) {
    let led = StatusLed::new(pin);
    spawner.spawn(status_led_task(led).unwrap());
}

struct StatusLed<'d> {
    pin: Output<'d>,
}

impl<'d> StatusLed<'d> {
    fn new(pin: GPIO46<'d>) -> Self {
        Self {
            pin: Output::new(pin, Level::Low, OutputConfig::default()),
        }
    }

    fn set_on(&mut self) {
        self.pin.set_high();
    }

    fn set_off(&mut self) {
        self.pin.set_low();
    }
}

#[embassy_executor::task]
pub async fn status_led_task(mut led: StatusLed<'static>) -> ! {
    info!("status_led:task started on GPIO46");

    let mut current = StatusLedRequest::OFF;
    let mut led_on = false;
    led.set_off();

    loop {
        match current.blink_period {
            None => {
                led_on = true;
                led.set_on();
                current = STATUS_LED_REQUEST_CHANNEL.receive().await;
            }
            Some(period) if period == Duration::from_millis(0) => {
                led_on = false;
                led.set_off();
                current = STATUS_LED_REQUEST_CHANNEL.receive().await;
            }
            Some(period) => {
                match select(STATUS_LED_REQUEST_CHANNEL.receive(), Timer::after(period)).await {
                    Either::First(request) => {
                        current = request;
                        led_on = true;
                    }
                    Either::Second(_) => {
                        led_on = !led_on;
                    }
                }

                if led_on {
                    led.set_on();
                } else {
                    led.set_off();
                }
            }
        }
    }
}
