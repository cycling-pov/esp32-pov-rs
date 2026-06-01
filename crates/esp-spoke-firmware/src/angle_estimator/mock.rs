#[cfg(feature = "mock-spin")]
use defmt::info;
#[cfg(feature = "mock-spin")]
use embassy_time::{Duration, Instant, Timer};
#[cfg(feature = "mock-spin")]
use pov_algs::{Angle, AngularVelocity};

#[cfg(feature = "mock-spin")]
use super::{SharedSpinState, SpinEstimator, SpinState};

/// Mock [`SpinEstimator`] that spins at a constant configurable rate.
///
/// Angular position is extrapolated from the moment of construction using
/// `embassy_time::Instant`. Useful for bench testing without hardware sensor.
///
/// Enabled by the `mock-spin` crate feature.
#[cfg(feature = "mock-spin")]
pub struct MockSpinEstimator {
    rate: AngularVelocity,
    start: Instant,
}

#[cfg(feature = "mock-spin")]
impl MockSpinEstimator {
    pub fn new(rate: AngularVelocity) -> Self {
        Self {
            rate,
            start: Instant::now(),
        }
    }
}

#[cfg(feature = "mock-spin")]
impl SpinEstimator for MockSpinEstimator {
    fn spin_state(&self) -> SpinState {
        let elapsed_us = self.start.elapsed().as_micros();
        let elapsed = Duration::from_micros(elapsed_us);
        SpinState {
            position: (self.rate * elapsed.into()).constrain_circle(),
            rate: self.rate,
        }
    }
}

/// Default mock spin rate used by [`mock_dual_spin_estimator_task`]: 2 revolutions per second.
#[cfg(feature = "mock-spin")]
pub const MOCK_SPIN_RATE: AngularVelocity =
    AngularVelocity::from_radians_secs(2.0 * core::f32::consts::TAU);

/// Mock phase offset applied to strip 1 in dual-strip mode.
///
/// In hardware, opposite spokes are approximately 180 degrees apart. Apply the
/// same relationship in mock mode so bench tests render opposite hemispheres.
#[cfg(feature = "mock-spin")]
pub const MOCK_STRIP1_PHASE_OFFSET: Angle = Angle::from_radians(core::f32::consts::PI);

/// Background task that drives both [`SharedSpinState`]s from a [`MockSpinEstimator`].
///
/// Drop-in replacement for [`dual_spin_estimator_task`] when the `mock-spin`
/// feature is active.  Both strips spin at [`MOCK_SPIN_RATE`].
#[cfg(feature = "mock-spin")]
#[embassy_executor::task]
pub async fn mock_dual_spin_estimator_task(
    state0: &'static SharedSpinState,
    state1: &'static SharedSpinState,
) -> ! {
    let mock0 = MockSpinEstimator::new(MOCK_SPIN_RATE);
    let mock1 = MockSpinEstimator::new(MOCK_SPIN_RATE);
    loop {
        Timer::after(Duration::from_millis(1)).await;

        if super::pause_needed_for_flash() {
            info!("spin:mock paused for flash write");
            super::spin_estimator_pause_spin();
            info!("spin:mock resumed after flash write");
            continue;
        }

        // let s0 = (&mock0 as &dyn SpinEstimator).spin_state();
        let s0 = mock0.spin_state();
        state0.lock(|s| *s.borrow_mut() = s0);
        let mut s1 = mock1.spin_state();
        s1.position = (s1.position + MOCK_STRIP1_PHASE_OFFSET).constrain_circle();
        state1.lock(|s| *s.borrow_mut() = s1);
    }
}
