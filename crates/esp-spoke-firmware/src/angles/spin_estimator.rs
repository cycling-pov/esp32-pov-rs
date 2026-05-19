use core::cell::RefCell;
use core::time::Duration;

use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration as EmbassyDuration, Instant, Timer};
use pov_algs::filters::PositionEstimator;
use pov_algs::{Angle, AngularVelocity};

/// Current rotational state of the spoke wheel.
#[derive(Clone, Copy)]
pub struct SpinState {
    /// Current angular position in the range [0, 2π).
    pub position: Angle,
    /// Current angular velocity in radians per second.
    pub rate: AngularVelocity,
}

impl Default for SpinState {
    fn default() -> Self {
        Self {
            position: Angle::from_radians(0.0),
            rate: AngularVelocity::from_radians_secs(0.0),
        }
    }
}

/// Shared spin state written by [`spin_estimator_task`] and read by consumers.
pub type SharedSpinState = BlockingMutex<CriticalSectionRawMutex, RefCell<SpinState>>;

/// Creates a const-initializable shared spin state, suitable for use in a `static`.
pub const fn new_shared_spin_state() -> SharedSpinState {
    BlockingMutex::new(RefCell::new(SpinState {
        position: Angle::from_radians(0.0),
        rate: AngularVelocity::from_radians_secs(0.0),
    }))
}

/// Signal written by hardware sensor tasks when a spoke passes the reference point.
/// Any write triggers a position update in [`spin_estimator_task`].
pub static SENSOR_TRIGGER: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Background task that maintains the spoke wheel position estimate.
///
/// Steps a [`PositionEstimator`] on a 1 ms tick. Callers with hardware ADC
/// access should call `SENSOR_TRIGGER.signal(())` from their own task whenever
/// the spoke passes the hall-effect sensor.
#[embassy_executor::task]
pub async fn spin_estimator_task(state: &'static SharedSpinState) -> ! {
    let mut estimator = PositionEstimator::<1>::default();
    let mut last = Instant::now();

    loop {
        Timer::after(EmbassyDuration::from_millis(1)).await;

        let now = Instant::now();
        let dt = Duration::from_micros(now.duration_since(last).as_micros());
        last = now;

        let triggered = SENSOR_TRIGGER.try_take().map(|_| 0usize);
        estimator.step(dt, triggered);

        state.lock(|s| {
            *s.borrow_mut() = SpinState {
                position: estimator.get_current_pos(),
                rate: estimator.get_current_rate(),
            };
        });
    }
}

/// Read-only access to the current rotational state.
pub trait SpinEstimator {
    fn spin_state(&self) -> SpinState;
}

/// [`SpinEstimator`] backed by the static state populated by [`spin_estimator_task`].
pub struct AdcSpinEstimator {
    state: &'static SharedSpinState,
}

impl AdcSpinEstimator {
    pub fn new(state: &'static SharedSpinState) -> Self {
        Self { state }
    }
}

impl SpinEstimator for AdcSpinEstimator {
    fn spin_state(&self) -> SpinState {
        self.state.lock(|s| *s.borrow())
    }
}

/// Mock [`SpinEstimator`] that spins at a constant configurable rate.
///
/// Angular position is extrapolated from the moment of construction using
/// `embassy_time::Instant`. Useful for bench testing without hardware sensor.
pub struct MockSpinEstimator {
    rate: AngularVelocity,
    start: Instant,
}

impl MockSpinEstimator {
    pub fn new(rate: AngularVelocity) -> Self {
        Self {
            rate,
            start: Instant::now(),
        }
    }
}

impl SpinEstimator for MockSpinEstimator {
    fn spin_state(&self) -> SpinState {
        let elapsed_us = self.start.elapsed().as_micros();
        let elapsed = Duration::from_micros(elapsed_us);
        SpinState {
            position: (self.rate * elapsed).constrain_circle(),
            rate: self.rate,
        }
    }
}
