#[cfg(all(feature = "imu-spin", not(feature = "bmi260")))]
compile_error!("`imu-spin` requires the `bmi260` IMU backend feature");

#[cfg(feature = "bmi260")]
mod bmi260;

#[cfg(feature = "hybrid-angle-estimator")]
pub use bmi260::subscribe_spin_rate;

#[cfg(feature = "hybrid-angle-estimator")]
#[embassy_executor::task]
pub async fn imu_spin_rate_publisher_task() -> ! {
    bmi260::spin_rate_publisher_impl().await
}

#[cfg(feature = "imu-spin")]
#[embassy_executor::task]
pub async fn imu_dual_spin_estimator_task(
    state0: &'static super::SharedSpinState,
    state1: &'static super::SharedSpinState,
    imu_offset_degrees: f32,
) -> ! {
    #[cfg(feature = "bmi260")]
    {
        bmi260::imu_dual_spin_estimator_impl(state0, state1, imu_offset_degrees).await
    }
}
