pub mod adc_monitor;
pub mod spin_estimator;

pub use spin_estimator::{
    AdcSpinEstimator, SENSOR_TRIGGER, SENSOR_TRIGGER_0, SENSOR_TRIGGER_1, SharedSpinState,
    SpinEstimator, SpinState, dual_spin_estimator_task, new_shared_spin_state, spin_estimator_task,
};
#[cfg(feature = "mock-spin")]
pub use spin_estimator::{MockSpinEstimator, mock_dual_spin_estimator_task};
