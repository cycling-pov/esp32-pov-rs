#![no_std]

#[cfg(all(feature = "ble", feature = "espnow", not(feature = "coexistence")))]
compile_error!(
    "features `ble` and `espnow` require `coexistence`; enable it to run both transports together"
);

#[cfg(all(feature = "imu-spin", feature = "mock-spin"))]
compile_error!("features `imu-spin` and `mock-spin` are mutually exclusive");

extern crate alloc;

pub mod angles;
pub mod bitmap;
pub mod led;
pub mod networking;
pub mod storage;
