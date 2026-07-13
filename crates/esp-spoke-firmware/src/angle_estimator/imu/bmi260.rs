use core::time::Duration;
use defmt::info;
#[cfg(feature = "hybrid-angle-estimator")]
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
#[cfg(feature = "hybrid-angle-estimator")]
use embassy_sync::pubsub::{PubSubChannel, Subscriber};
use embassy_time::{Duration as EmbassyDuration, Instant, Timer};
use nalgebra::RealField;
use pov_algs::{Angle, AngularVelocity};

const IMU_SAMPLE_RATE_WARNING_THRESHOLD: EmbassyDuration = EmbassyDuration::from_hz(5);

#[cfg(feature = "hybrid-angle-estimator")]
const IMU_SPIN_RATE_CAPACITY: usize = 16;
#[cfg(feature = "hybrid-angle-estimator")]
const IMU_SPIN_RATE_SUBSCRIBERS: usize = 2;
#[cfg(feature = "hybrid-angle-estimator")]
const IMU_SPIN_RATE_PUBLISHERS: usize = 1;

#[cfg(feature = "hybrid-angle-estimator")]
const HYBRID_SAMPLE_RATE: EmbassyDuration = EmbassyDuration::from_hz(20);

struct CalibrationData {
    gyro_bias_dps: nalgebra::Vector3<f32>,
    calibrating_gyro_bias: bool,
    calibration_accum_dps: nalgebra::Vector3<f32>,
    calibration_elapsed: Duration,
    calibration_samples: u32,
    calibration_reset_log_divider: u8,
}

struct SampleRateMonitor {
    sample_counter: u32,
    sample_time_accum: Duration,
}

#[cfg(feature = "hybrid-angle-estimator")]
pub type SpinRateSubscriber = Subscriber<
    'static,
    CriticalSectionRawMutex,
    AngularVelocity,
    IMU_SPIN_RATE_CAPACITY,
    IMU_SPIN_RATE_SUBSCRIBERS,
    IMU_SPIN_RATE_PUBLISHERS,
>;

#[cfg(feature = "hybrid-angle-estimator")]
static IMU_SPIN_RATE_SAMPLES: PubSubChannel<
    CriticalSectionRawMutex,
    AngularVelocity,
    IMU_SPIN_RATE_CAPACITY,
    IMU_SPIN_RATE_SUBSCRIBERS,
    IMU_SPIN_RATE_PUBLISHERS,
> = PubSubChannel::new();

fn check_sample_rate(monitor: &mut SampleRateMonitor, dt: Duration) {
    monitor.sample_counter = monitor.sample_counter.wrapping_add(1);
    monitor.sample_time_accum += dt;
    if monitor.sample_counter >= 500 {
        let elapsed_s = monitor.sample_time_accum.as_secs_f32().max(1e-6);
        let hz = (monitor.sample_counter as f32 / elapsed_s) as u64;
        let min_hz = 1000 / IMU_SAMPLE_RATE_WARNING_THRESHOLD.as_millis();

        if hz < min_hz {
            defmt::warn!("spin:imu low sample rate hz={=u64}", hz);
        } else {
            defmt::debug!("spin:imu sample rate hz={=u64}", hz);
        }
        monitor.sample_counter = 0;
        monitor.sample_time_accum = Duration::ZERO;
    }
}

#[cfg(feature = "hybrid-angle-estimator")]
fn dominant_signed_rate_dps(gyro_dps: nalgebra::Vector3<f32>) -> f32 {
    const IMU_ANGLE_DIRECTION: f32 = -1.0;

    let dominant_axis_rate_dps =
        if gyro_dps.x.abs() >= gyro_dps.y.abs() && gyro_dps.x.abs() >= gyro_dps.z.abs() {
            gyro_dps.x
        } else if gyro_dps.y.abs() >= gyro_dps.z.abs() {
            gyro_dps.y
        } else {
            gyro_dps.z
        };

    IMU_ANGLE_DIRECTION * dominant_axis_rate_dps
}

#[cfg(feature = "hybrid-angle-estimator")]
pub fn subscribe_spin_rate() -> Option<SpinRateSubscriber> {
    IMU_SPIN_RATE_SAMPLES.subscriber().ok()
}

#[cfg(feature = "hybrid-angle-estimator")]
fn publish_spin_rate(rate: AngularVelocity) {
    IMU_SPIN_RATE_SAMPLES
        .immediate_publisher()
        .publish_immediate(rate);
}

#[cfg(feature = "hybrid-angle-estimator")]
fn check_and_initialize_gyro_bias_rate_only(
    calibration_data: &mut CalibrationData,
    sample: &crate::imu::ImuSample,
    dt: Duration,
) -> bool {
    const IMU_CALIBRATION_DURATION: Duration = Duration::from_secs(5);
    const IMU_CALIBRATION_MOTION_MAX_DPS: f32 = 100.0;

    if !calibration_data.calibrating_gyro_bias {
        return false;
    }

    let gyro_norm_dps = sample.gyro_dps.norm();
    if gyro_norm_dps <= IMU_CALIBRATION_MOTION_MAX_DPS {
        calibration_data.calibration_accum_dps += sample.gyro_dps;
        calibration_data.calibration_elapsed += dt;
        calibration_data.calibration_samples = calibration_data.calibration_samples.wrapping_add(1);

        if calibration_data.calibration_elapsed >= IMU_CALIBRATION_DURATION
            && calibration_data.calibration_samples > 0
        {
            let inv_n = 1.0 / calibration_data.calibration_samples as f32;
            calibration_data.gyro_bias_dps = calibration_data.calibration_accum_dps * inv_n;
            calibration_data.calibrating_gyro_bias = false;
            super::super::publish_imu_boot_calibrating(false);
            info!(
                "spin:hybrid gyro bias calibrated dps=({=f32}, {=f32}, {=f32})",
                calibration_data.gyro_bias_dps.x,
                calibration_data.gyro_bias_dps.y,
                calibration_data.gyro_bias_dps.z,
            );
        }
    } else {
        calibration_data.calibration_accum_dps = nalgebra::Vector3::new(0.0f32, 0.0, 0.0);
        calibration_data.calibration_elapsed = Duration::ZERO;
        calibration_data.calibration_samples = 0;
        calibration_data.calibration_reset_log_divider = calibration_data
            .calibration_reset_log_divider
            .wrapping_add(1);
        if calibration_data.calibration_reset_log_divider == 0 {
            defmt::warn!(
                "spin:hybrid calibration reset; motion detected dps={=f32}",
                gyro_norm_dps
            );
        }
    }

    true
}

#[cfg(feature = "imu-spin")]
fn check_and_initialize_gyro_bias(
    calibration_data: &mut CalibrationData,
    sample: &crate::imu::ImuSample,
    dt: Duration,
    last_angle: Angle,
    state0: &super::super::SharedSpinState,
    state1: &super::super::SharedSpinState,
) -> bool {
    const IMU_CALIBRATION_DURATION: Duration = Duration::from_secs(5);
    const IMU_CALIBRATION_MOTION_MAX_DPS: f32 = 100.0;

    if !calibration_data.calibrating_gyro_bias {
        return false;
    }

    let gyro_norm_dps = sample.gyro_dps.norm();
    if gyro_norm_dps <= IMU_CALIBRATION_MOTION_MAX_DPS {
        calibration_data.calibration_accum_dps += sample.gyro_dps;
        calibration_data.calibration_elapsed += dt;
        calibration_data.calibration_samples = calibration_data.calibration_samples.wrapping_add(1);

        if calibration_data.calibration_elapsed >= IMU_CALIBRATION_DURATION
            && calibration_data.calibration_samples > 0
        {
            let inv_n = 1.0 / calibration_data.calibration_samples as f32;
            calibration_data.gyro_bias_dps = calibration_data.calibration_accum_dps * inv_n;
            calibration_data.calibrating_gyro_bias = false;
            super::super::publish_imu_boot_calibrating(false);
            info!(
                "spin:imu gyro bias calibrated dps=({=f32}, {=f32}, {=f32})",
                calibration_data.gyro_bias_dps.x,
                calibration_data.gyro_bias_dps.y,
                calibration_data.gyro_bias_dps.z,
            );
        }
    } else {
        calibration_data.calibration_accum_dps = nalgebra::Vector3::new(0.0f32, 0.0, 0.0);
        calibration_data.calibration_elapsed = Duration::ZERO;
        calibration_data.calibration_samples = 0;
        calibration_data.calibration_reset_log_divider = calibration_data
            .calibration_reset_log_divider
            .wrapping_add(1);
        if calibration_data.calibration_reset_log_divider == 0 {
            defmt::warn!(
                "spin:imu calibration reset; motion detected dps={=f32}",
                gyro_norm_dps
            );
        }
    }

    let zero_rate = AngularVelocity::from_radians_secs(0.0);
    state0.lock(|s| {
        *s.borrow_mut() = super::super::SpinState {
            position: last_angle,
            rate: zero_rate,
        };
    });
    state1.lock(|s| {
        *s.borrow_mut() = super::super::SpinState {
            position: last_angle,
            rate: zero_rate,
        };
    });

    true
}

#[cfg(feature = "imu-spin")]
pub async fn imu_dual_spin_estimator_impl(
    state0: &'static super::super::SharedSpinState,
    state1: &'static super::super::SharedSpinState,
    imu_offset_degrees: f32,
) -> ! {
    use fusion_ahrs::{Ahrs, AhrsSettings, Convention};

    const GYRO_AXIS_MIN_RATE_DPS: f32 = 30.0;
    const GRAVITY_PROJECTION_MIN_NORM: f32 = 0.2;
    const IMU_ANGLE_DIRECTION: f32 = -1.0;
    const STRIP0_PHASE_OFFSET_FROM_SENSOR: Angle = Angle::from_degrees(90.0);
    const STRIP1_PHASE_OFFSET_FROM_SENSOR: Angle = Angle::from_degrees(-90.0);
    let imu_offset = Angle::from_degrees(imu_offset_degrees);

    let settings = AhrsSettings {
        convention: Convention::Nwu,
        gain: 0.50,
        gyroscope_range: 2000.0,
        acceleration_rejection: 15.0,
        recovery_trigger_period: 1000,
        magnetic_rejection: 15.0,
    };

    let mut ahrs = Ahrs::with_settings(settings);
    let mut samples =
        crate::imu::subscribe().expect("imu sample subscriber unavailable for imu-spin estimator");
    let mut last = Instant::now();
    let mut last_angle = Angle::from_radians(0.0);

    let mut calibration_data = CalibrationData {
        gyro_bias_dps: nalgebra::Vector3::new(0.0f32, 0.0, 0.0),
        calibrating_gyro_bias: true,
        calibration_accum_dps: nalgebra::Vector3::new(0.0f32, 0.0, 0.0),
        calibration_elapsed: Duration::ZERO,
        calibration_samples: 0,
        calibration_reset_log_divider: 0,
    };
    super::super::publish_imu_boot_calibrating(true);

    // Body-frame reference direction captured once after AHRS convergence.
    // Equals world-up projected onto the spin plane in body frame, so the
    // output angle is 0 when that direction points toward world-up.
    let mut ref_body: Option<nalgebra::Vector3<f32>> = None;

    loop {
        let sample = samples.next_message_pure().await;

        let now = Instant::now();
        let dt = Duration::from_micros(now.duration_since(last).as_micros());
        last = now;

        if check_and_initialize_gyro_bias(
            &mut calibration_data,
            &sample,
            dt,
            last_angle,
            state0,
            state1,
        ) {
            continue;
        }

        let corrected_gyro_dps = sample.gyro_dps - calibration_data.gyro_bias_dps;
        ahrs.update_no_magnetometer(corrected_gyro_dps, sample.accel_g, dt.as_secs_f32());

        let gyro_rate_dps = corrected_gyro_dps.norm();
        let dominant_axis_rate_dps = if corrected_gyro_dps.x.abs() >= corrected_gyro_dps.y.abs()
            && corrected_gyro_dps.x.abs() >= corrected_gyro_dps.z.abs()
        {
            corrected_gyro_dps.x
        } else if corrected_gyro_dps.y.abs() >= corrected_gyro_dps.z.abs() {
            corrected_gyro_dps.y
        } else {
            corrected_gyro_dps.z
        };
        let signed_rate_dps = IMU_ANGLE_DIRECTION * dominant_axis_rate_dps;
        let rate = AngularVelocity::from_degrees_secs(signed_rate_dps);

        let q = ahrs.quaternion();
        // NWU convention: world Z = up.
        let world_up = nalgebra::Vector3::new(0.0f32, 0.0, 1.0);
        let mut updated_from_geometry = false;

        if gyro_rate_dps >= GYRO_AXIS_MIN_RATE_DPS {
            // Spin axis in body frame, signed for rotation direction.
            let spin_body = corrected_gyro_dps / gyro_rate_dps * IMU_ANGLE_DIRECTION;
            // Spin axis in world frame.
            let spin_world = q * spin_body;

            // Project world-up onto the spin plane (world frame).
            let up_raw = world_up - spin_world * world_up.dot(&spin_world);
            let up_norm = up_raw.norm();

            if up_norm >= GRAVITY_PROJECTION_MIN_NORM {
                // Orthonormal basis for the spin plane in world frame:
                //   up_in_spin_frame  - direction of world-up within the plane
                //   e2 - 90 degrees ahead in the direction of rotation
                let up_in_spin_frame = up_raw / up_norm;
                let e2 = spin_world.cross(&up_in_spin_frame);

                // Capture body-frame reference once AHRS has converged.
                if ref_body.is_none() && !ahrs.flags().initialising {
                    let up_in_body = q.inverse() * world_up;
                    let up_body_raw = up_in_body - spin_body * up_in_body.dot(&spin_body);
                    let up_body_norm = up_body_raw.norm();
                    if up_body_norm >= GRAVITY_PROJECTION_MIN_NORM {
                        ref_body = Some(up_body_raw / up_body_norm);
                        info!("spin:imu angle reference initialized");
                    }
                }

                if let Some(ref_b) = ref_body {
                    // Rotate body-fixed reference into world frame.
                    let ref_world = q * ref_b;
                    let ref_perp = ref_world - spin_world * ref_world.dot(&spin_world);
                    // angle = 0 when ref_b aligns with world-up;
                    // increases monotonically in the rotation direction.
                    last_angle = Angle::from_radians(
                        ref_perp.dot(&e2).atan2(ref_perp.dot(&up_in_spin_frame)),
                    )
                    .constrain_circle();
                    updated_from_geometry = true;
                }
            }
        }

        // Keep phase moving when geometric projection is unavailable.
        if !updated_from_geometry {
            let delta_angle = Angle::from_degrees(signed_rate_dps * dt.as_secs_f32());
            last_angle = (last_angle + delta_angle).constrain_circle();
        }

        let strip0_angle = (last_angle + STRIP0_PHASE_OFFSET_FROM_SENSOR + imu_offset)
            .constrain_circle();

        let strip1_angle = (last_angle + STRIP1_PHASE_OFFSET_FROM_SENSOR + imu_offset)
            .constrain_circle();

        state0.lock(|s| {
            *s.borrow_mut() = super::super::SpinState {
                position: strip0_angle,
                rate,
            };
        });
        state1.lock(|s| {
            *s.borrow_mut() = super::super::SpinState {
                position: strip1_angle,
                rate,
            };
        });
    }
}

#[cfg(feature = "hybrid-angle-estimator")]
pub async fn spin_rate_publisher_impl() -> ! {
    super::super::publish_imu_boot_calibrating(true);

    let mut samples =
        crate::imu::subscribe().expect("imu sample subscriber unavailable for hybrid estimator");
    let mut sample_rate_monitor = SampleRateMonitor {
        sample_counter: 0,
        sample_time_accum: Duration::ZERO,
    };
    let mut calibration_data = CalibrationData {
        gyro_bias_dps: nalgebra::Vector3::new(0.0f32, 0.0, 0.0),
        calibrating_gyro_bias: true,
        calibration_accum_dps: nalgebra::Vector3::new(0.0f32, 0.0, 0.0),
        calibration_elapsed: Duration::ZERO,
        calibration_samples: 0,
        calibration_reset_log_divider: 0,
    };

    // Wait for the first sample so startup timing is aligned to sensor output.
    let mut sample = samples.next_message_pure().await;
    let mut last = Instant::now();

    loop {
        Timer::after(HYBRID_SAMPLE_RATE).await;

        while let Some(next) = samples.try_next_message_pure() {
            sample = next;
        }

        let now = Instant::now();
        let dt = Duration::from_micros(now.duration_since(last).as_micros());
        last = now;

        if check_and_initialize_gyro_bias_rate_only(&mut calibration_data, &sample, dt) {
            continue;
        }

        let corrected_gyro_dps = sample.gyro_dps - calibration_data.gyro_bias_dps;
        let rate = AngularVelocity::from_degrees_secs(dominant_signed_rate_dps(corrected_gyro_dps));
        publish_spin_rate(rate);
        check_sample_rate(&mut sample_rate_monitor, dt);
    }
}
