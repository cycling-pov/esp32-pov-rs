#[cfg(all(feature = "imu-spin", feature = "adafruit-9dof", feature = "bmi260"))]
compile_error!(
    "Select exactly one IMU backend: enable either `adafruit-9dof` or `bmi260`, not both"
);

#[cfg(all(
    feature = "imu-spin",
    not(any(feature = "adafruit-9dof", feature = "bmi260"))
))]
compile_error!("`imu-spin` requires one IMU backend feature: `adafruit-9dof` or `bmi260`");

#[cfg(all(feature = "imu-spin", feature = "adafruit-9dof"))]
mod adafruit_9dof;
#[cfg(all(feature = "imu-spin", feature = "bmi260"))]
mod bmi260;

#[cfg(feature = "imu-spin")]
type SharedI2cDevice = embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice<
    'static,
    embassy_sync::blocking_mutex::raw::NoopRawMutex,
    esp_hal::i2c::master::I2c<'static, esp_hal::Async>,
>;

#[cfg(feature = "hybrid-angle-estimator")]
pub use bmi260::subscribe_spin_rate;

#[cfg(feature = "hybrid-angle-estimator")]
#[embassy_executor::task]
pub async fn imu_spin_rate_publisher_task(i2c: SharedI2cDevice) -> ! {
    bmi260::spin_rate_publisher_impl(i2c).await
}

#[cfg(feature = "imu-spin")]
#[embassy_executor::task]
pub async fn imu_dual_spin_estimator_task(
    state0: &'static super::SharedSpinState,
    state1: &'static super::SharedSpinState,
    i2c: SharedI2cDevice,
    imu_offset_degrees: f32,
) -> ! {
    #[cfg(feature = "adafruit-9dof")]
    {
        adafruit_9dof::imu_dual_spin_estimator_impl(state0, state1, i2c, imu_offset_degrees).await
    }

    #[cfg(feature = "bmi260")]
    {
        bmi260::imu_dual_spin_estimator_impl(state0, state1, i2c, imu_offset_degrees).await
    }
}
