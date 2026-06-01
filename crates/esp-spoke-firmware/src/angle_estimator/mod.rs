pub mod hall_effect;
pub mod imu;
pub mod mock;

use core::cell::RefCell;
use core::sync::atomic::Ordering;

use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use pov_algs::{Angle, AngularVelocity};

use crate::led::{CORE1_FLASH_PAUSE_REQUESTED, CORE1_FLASH_PAUSED_COUNT};

#[cfg(feature = "imu-spin")]
pub use imu::imu_dual_spin_estimator_task;
#[cfg(feature = "mock-spin")]
pub use mock::mock_dual_spin_estimator_task;

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

pub trait SpinEstimator {
    fn spin_state(&self) -> SpinState;
}

/// Busy-spins in IRAM while flash is being written.
///
/// Placed in IRAM via `#[esp_hal::ram]` so no flash-backed ICache pages are
/// referenced during the spin. Symmetric with `render_pause_spin` in
/// `pov_dual_strip`.
#[esp_hal::ram]
fn spin_estimator_pause_spin() {
    CORE1_FLASH_PAUSED_COUNT.fetch_add(1, Ordering::Release);
    while CORE1_FLASH_PAUSE_REQUESTED.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }
    CORE1_FLASH_PAUSED_COUNT.fetch_sub(1, Ordering::Release);
}

fn pause_needed_for_flash() -> bool {
    CORE1_FLASH_PAUSE_REQUESTED.load(Ordering::Acquire)
}
