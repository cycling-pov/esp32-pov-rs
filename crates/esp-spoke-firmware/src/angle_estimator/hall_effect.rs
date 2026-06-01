use core::time::Duration;

use defmt::info;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration as EmbassyDuration, Instant, Timer};
use pov_algs::filters::PositionEstimator;

/// Signal written by hardware sensor tasks when a spoke passes the reference point.
/// Any write triggers a position update in [`spin_estimator_task`].
pub static SENSOR_TRIGGER: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Per-strip sensor signals for dual-strip POV mode.
/// Strip 0's hall-effect sensor task calls `SENSOR_TRIGGER_0.signal(())`;
/// strip 1's sensor task calls `SENSOR_TRIGGER_1.signal(())`.  Both are
/// consumed by [`dual_spin_estimator_task`].
pub static SENSOR_TRIGGER_0: Signal<CriticalSectionRawMutex, ()> = Signal::new();
pub static SENSOR_TRIGGER_1: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Background task that maintains the spoke wheel position estimate.
///
/// Steps a [`PositionEstimator`] on a 1 ms tick. Callers with hardware ADC
/// access should call `SENSOR_TRIGGER.signal(())` from their own task whenever
/// the spoke passes the hall-effect sensor.
#[embassy_executor::task]
pub async fn spin_estimator_task(state: &'static super::SharedSpinState) -> ! {
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
            *s.borrow_mut() = super::SpinState {
                position: estimator.get_current_pos(),
                rate: estimator.get_current_rate(),
            };
        });
    }
}

/// Background task that independently tracks the angular position of two
/// strips, each with its own hall-effect sensor.
///
/// Both strips are assumed to be on the same spinning wheel (same RPM) but
/// each sensor provides its own zero/phase reference.  Two separate
/// [`PositionEstimator`] instances are stepped on a 1 ms tick; each is
/// triggered by its own sensor signal ([`SENSOR_TRIGGER_0`] /
/// [`SENSOR_TRIGGER_1`]) and writes to its own [`super::SharedSpinState`].
#[embassy_executor::task]
pub async fn dual_spin_estimator_task(
    state0: &'static super::SharedSpinState,
    state1: &'static super::SharedSpinState,
) -> ! {
    let mut estimator0 = PositionEstimator::<1>::default();
    let mut estimator1 = PositionEstimator::<1>::default();
    let mut last = Instant::now();

    loop {
        Timer::after(EmbassyDuration::from_millis(1)).await;

        if super::pause_needed_for_flash() {
            info!("spin:dual paused for flash write");
            super::spin_estimator_pause_spin();
            info!("spin:dual resumed after flash write");
            continue;
        }

        let now = Instant::now();
        let dt = Duration::from_micros(now.duration_since(last).as_micros());
        last = now;

        let triggered0 = SENSOR_TRIGGER_0.try_take().map(|_| 0usize);
        estimator0.step(dt, triggered0);
        state0.lock(|s| {
            *s.borrow_mut() = super::SpinState {
                position: estimator0.get_current_pos(),
                rate: estimator0.get_current_rate(),
            };
        });

        let triggered1 = SENSOR_TRIGGER_1.try_take().map(|_| 0usize);
        estimator1.step(dt, triggered1);
        state1.lock(|s| {
            *s.borrow_mut() = super::SpinState {
                position: estimator1.get_current_pos(),
                rate: estimator1.get_current_rate(),
            };
        });
    }
}
