#![no_std]

#[cfg(all(feature = "ble", feature = "espnow", not(feature = "coexistence")))]
compile_error!(
    "features `ble` and `espnow` require `coexistence`; enable it to run both transports together"
);

#[cfg(all(feature = "pure-imu-angle-estimator", feature = "mock-spin"))]
compile_error!("features `pure-imu-angle-estimator` and `mock-spin` are mutually exclusive");

extern crate alloc;

#[cfg(feature = "adc")]
pub mod adc;
pub mod angle_estimator;
pub mod bitmap;
#[cfg(feature = "bmi260")]
pub mod imu;
pub mod led;
pub mod networking;
pub mod pushbutton;
#[cfg(feature = "status-led")]
pub mod status_led;
pub mod storage;
