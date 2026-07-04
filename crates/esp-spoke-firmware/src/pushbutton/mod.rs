use defmt::{info, warn};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::{PubSubChannel, Subscriber};
use embassy_time::{Duration, Instant, Timer};
use esp_hal::gpio::{AnyPin, Input, InputConfig, Pull};

const BUTTON_DEBOUNCE_INTERVAL: Duration = Duration::from_millis(30);
const BUTTON_EVENT_CAPACITY: usize = 8;
const BUTTON_EVENT_SUBSCRIBERS: usize = 4;
const BUTTON_EVENT_PUBLISHERS: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq, defmt::Format)]
pub enum ButtonId {
    Button1,
    Button2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, defmt::Format)]
pub enum ButtonEvent {
    Pressed(ButtonId),
    Released { button: ButtonId, held: Duration },
}

pub type ButtonEventSubscriber = Subscriber<
    'static,
    CriticalSectionRawMutex,
    ButtonEvent,
    BUTTON_EVENT_CAPACITY,
    BUTTON_EVENT_SUBSCRIBERS,
    BUTTON_EVENT_PUBLISHERS,
>;

static BUTTON_EVENTS: PubSubChannel<
    CriticalSectionRawMutex,
    ButtonEvent,
    BUTTON_EVENT_CAPACITY,
    BUTTON_EVENT_SUBSCRIBERS,
    BUTTON_EVENT_PUBLISHERS,
> = PubSubChannel::new();

/// Register a new button event subscriber.
///
/// Returns `None` when all subscriber slots are occupied.
pub fn subscribe() -> Option<ButtonEventSubscriber> {
    BUTTON_EVENTS.subscriber().ok()
}

fn emit_pressed(button: ButtonId) {
    BUTTON_EVENTS
        .immediate_publisher()
        .publish_immediate(ButtonEvent::Pressed(button));
}

fn emit_released(button: ButtonId, held: Duration) {
    BUTTON_EVENTS
        .immediate_publisher()
        .publish_immediate(ButtonEvent::Released { button, held });
}

#[embassy_executor::task(pool_size = 2)]
pub async fn button_input_task(pin: AnyPin<'static>, button: ButtonId) -> ! {
    let config = InputConfig::default().with_pull(Pull::Up);
    let mut input = Input::new(pin, config);

    info!("button:{:?} task started", button);

    loop {
        input.wait_for_falling_edge().await;

        // Re-sample after the debounce interval to filter contact bounce.
        Timer::after(BUTTON_DEBOUNCE_INTERVAL).await;

        if !input.is_low() {
            continue;
        }

        let pressed_at = Instant::now();
        info!("button:{:?} pressed", button);
        emit_pressed(button);

        // Wait for stable release to avoid duplicate press events while held.
        input.wait_for_rising_edge().await;
        Timer::after(BUTTON_DEBOUNCE_INTERVAL).await;

        if input.is_low() {
            warn!("button:{:?} release bounce detected", button);
            continue;
        }

        let held = Instant::now() - pressed_at;
        info!("button:{:?} released held={:?}", button, held);
        emit_released(button, held);
    }
}
