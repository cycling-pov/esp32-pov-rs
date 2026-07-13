use core::time::Duration;

use defmt::info;
use embassy_time::Instant;
use pov_algs::{Angle, filters::PositionEstimatorGyro};

use crate::adc::{self, AdcDevice, AdcSampleSource, AdcSelection};

#[embassy_executor::task]
pub async fn hybrid_dual_spin_estimator_task(
    state0: &'static super::SharedSpinState,
    state1: &'static super::SharedSpinState,
    hall_offset_0_degrees: f32,
    hall_offset_1_degrees: f32,
) -> ! {
    let hall_offset_0 = Angle::from_degrees(hall_offset_0_degrees);
    let hall_offset_1 = Angle::from_degrees(hall_offset_1_degrees);
    let hall_threshold = crate::storage::get_hybrid_hall_trigger_threshold().await;
    let mut hall_samples = adc::subscribe().expect("adc subscriber unavailable");
    let mut imu_samples =
        crate::imu::subscribe_spin_rate().expect("imu spin-rate subscriber unavailable");

    adc::start_monitor_mode(AdcSelection {
        board_rev: None,
        hall_effect_sensor2: Some(true),
        battery_voltage: None,
        hall_effect_sensor1: Some(true),
    })
    .await;

    let mut last = Instant::now();
    let mut estimator = PositionEstimatorGyro::<2>::new(Duration::from_secs(1));
    // Track whether each hall sensor is currently inside the trigger window.
    // This lets us emit one trigger per threshold-crossing edge.
    let mut hall_active = [false; 2];

    info!("spin:hybrid starting hall_threshold={=u16}", hall_threshold);

    loop {
        let rate = imu_samples.next_message_pure().await;

        let now = Instant::now();
        let dt_micros = now.duration_since(last).as_micros();
        let dt = Duration::from_micros(dt_micros);
        last = now;

        let mut triggered = None;
        while let Some(sample) = hall_samples.try_next_message_pure() {
            if sample.source != AdcSampleSource::Monitor {
                continue;
            }

            let sensor_index = match sample.device {
                AdcDevice::HallEffectSensor1 => Some(0usize),
                AdcDevice::HallEffectSensor2 => Some(1usize),
                _ => None,
            };

            let Some(sensor_index) = sensor_index else {
                continue;
            };

            let active = sample.raw < hall_threshold;
            if active && !hall_active[sensor_index] {
                triggered = Some(sensor_index);
            }
            hall_active[sensor_index] = active;
        }

        estimator.step(dt, rate, triggered);

        let position = estimator.get_current_pos();
        let rate = estimator.get_current_rate();

        state0.lock(|s| {
            *s.borrow_mut() = super::SpinState {
                position: (position + hall_offset_0).constrain_circle(),
                rate,
            };
        });
        state1.lock(|s| {
            *s.borrow_mut() = super::SpinState {
                position: (position + hall_offset_1).constrain_circle(),
                rate,
            };
        });
    }
}
