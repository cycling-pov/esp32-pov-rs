#![no_std]

#[cfg(all(feature = "ble", feature = "espnow", not(feature = "coexistence")))]
compile_error!(
    "features `ble` and `espnow` require `coexistence`; enable it to run both transports together"
);

extern crate alloc;

pub mod angles;
pub mod bitmap;
pub mod led;
pub mod networking;
