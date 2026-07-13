#![no_std]

#[cfg(all(feature = "ble", feature = "espnow", not(feature = "coexistence")))]
compile_error!(
    "features `ble` and `espnow` require `coexistence`; enable it to run both transports together"
);

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
