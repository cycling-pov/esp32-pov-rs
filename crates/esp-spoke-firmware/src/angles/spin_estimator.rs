use core::cell::RefCell;
use core::sync::atomic::Ordering;
use core::time::Duration;

use defmt::info;
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration as EmbassyDuration, Instant, Timer};
#[cfg(feature = "imu-spin")]
use nalgebra::RealField;
use pov_algs::filters::PositionEstimator;
use pov_algs::{Angle, AngularVelocity};

use crate::led::{CORE1_FLASH_PAUSE_REQUESTED, CORE1_FLASH_PAUSED_COUNT};

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

/// Signal written by hardware sensor tasks when a spoke passes the reference point.
/// Any write triggers a position update in [`spin_estimator_task`].
pub static SENSOR_TRIGGER: Signal<CriticalSectionRawMutex, ()> = Signal::new();

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

/// Per-strip sensor signals for dual-strip POV mode.
/// Strip 0's hall-effect sensor task calls `SENSOR_TRIGGER_0.signal(())`;
/// strip 1's sensor task calls `SENSOR_TRIGGER_1.signal(())`.  Both are
/// consumed by [`dual_spin_estimator_task`].
pub static SENSOR_TRIGGER_0: Signal<CriticalSectionRawMutex, ()> = Signal::new();
pub static SENSOR_TRIGGER_1: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Background task that maintains the spoke wheel position estimate.
///
/// Steps a [`PositionEstimator`] on a 1 ms tick. Callers with hardware ADC
/// access should call `SENSOR_TRIGGER.signal(())` from their own task whenever
/// the spoke passes the hall-effect sensor.
#[embassy_executor::task]
pub async fn spin_estimator_task(state: &'static SharedSpinState) -> ! {
    let mut estimator = PositionEstimator::<1>::default();
    let mut last = Instant::now();

    loop {
        Timer::after(EmbassyDuration::from_millis(1)).await;

        let now = Instant::now();
        let dt = Duration::from_micros(now.duration_since(last).as_micros());
        last = now;

        let triggered = SENSOR_TRIGGER.try_take().map(|_| 0usize);
        estimator.step(dt, triggered);

        state.lock(|s| {
            *s.borrow_mut() = SpinState {
                position: estimator.get_current_pos(),
                rate: estimator.get_current_rate(),
            };
        });
    }
}

/// Background task that independently tracks the angular position of two
/// strips, each with its own hall-effect sensor.
///
/// Both strips are assumed to be on the same spinning wheel (same RPM) but
/// each sensor provides its own zero/phase reference.  Two separate
/// [`PositionEstimator`] instances are stepped on a 1 ms tick; each is
/// triggered by its own sensor signal ([`SENSOR_TRIGGER_0`] /
/// [`SENSOR_TRIGGER_1`]) and writes to its own [`SharedSpinState`].
#[embassy_executor::task]
pub async fn dual_spin_estimator_task(
    state0: &'static SharedSpinState,
    state1: &'static SharedSpinState,
) -> ! {
    let mut estimator0 = PositionEstimator::<1>::default();
    let mut estimator1 = PositionEstimator::<1>::default();
    let mut last = Instant::now();

    loop {
        Timer::after(EmbassyDuration::from_millis(1)).await;

        if CORE1_FLASH_PAUSE_REQUESTED.load(Ordering::Acquire) {
            info!("spin:dual paused for flash write");
            spin_estimator_pause_spin();
            info!("spin:dual resumed after flash write");
            continue;
        }

        let now = Instant::now();
        let dt = Duration::from_micros(now.duration_since(last).as_micros());
        last = now;

        let triggered0 = SENSOR_TRIGGER_0.try_take().map(|_| 0usize);
        estimator0.step(dt, triggered0);
        state0.lock(|s| {
            *s.borrow_mut() = SpinState {
                position: estimator0.get_current_pos(),
                rate: estimator0.get_current_rate(),
            };
        });

        let triggered1 = SENSOR_TRIGGER_1.try_take().map(|_| 0usize);
        estimator1.step(dt, triggered1);
        state1.lock(|s| {
            *s.borrow_mut() = SpinState {
                position: estimator1.get_current_pos(),
                rate: estimator1.get_current_rate(),
            };
        });
    }
}

/// Read-only access to the current rotational state.
pub trait SpinEstimator {
    fn spin_state(&self) -> SpinState;
}

/// [`SpinEstimator`] backed by the static state populated by [`spin_estimator_task`].
pub struct AdcSpinEstimator {
    state: &'static SharedSpinState,
}

impl AdcSpinEstimator {
    pub fn new(state: &'static SharedSpinState) -> Self {
        Self { state }
    }
}

impl SpinEstimator for AdcSpinEstimator {
    fn spin_state(&self) -> SpinState {
        self.state.lock(|s| *s.borrow())
    }
}

/// Mock [`SpinEstimator`] that spins at a constant configurable rate.
///
/// Angular position is extrapolated from the moment of construction using
/// `embassy_time::Instant`. Useful for bench testing without hardware sensor.
///
/// Enabled by the `mock-spin` crate feature.
#[cfg(feature = "mock-spin")]
pub struct MockSpinEstimator {
    rate: AngularVelocity,
    start: Instant,
}

#[cfg(feature = "mock-spin")]
impl MockSpinEstimator {
    pub fn new(rate: AngularVelocity) -> Self {
        Self {
            rate,
            start: Instant::now(),
        }
    }
}

#[cfg(feature = "mock-spin")]
impl SpinEstimator for MockSpinEstimator {
    fn spin_state(&self) -> SpinState {
        let elapsed_us = self.start.elapsed().as_micros();
        let elapsed = Duration::from_micros(elapsed_us);
        SpinState {
            position: (self.rate * elapsed).constrain_circle(),
            rate: self.rate,
        }
    }
}

/// Default mock spin rate used by [`mock_dual_spin_estimator_task`]: 2 revolutions per second.
#[cfg(feature = "mock-spin")]
pub const MOCK_SPIN_RATE: AngularVelocity =
    AngularVelocity::from_radians_secs(2.0 * core::f32::consts::TAU);

/// Mock phase offset applied to strip 1 in dual-strip mode.
///
/// In hardware, opposite spokes are approximately 180 degrees apart. Apply the
/// same relationship in mock mode so bench tests render opposite hemispheres.
#[cfg(feature = "mock-spin")]
pub const MOCK_STRIP1_PHASE_OFFSET: Angle = Angle::from_radians(core::f32::consts::PI);

/// Background task that drives both [`SharedSpinState`]s from a [`MockSpinEstimator`].
///
/// Drop-in replacement for [`dual_spin_estimator_task`] when the `mock-spin`
/// feature is active.  Both strips spin at [`MOCK_SPIN_RATE`].
#[cfg(feature = "mock-spin")]
#[embassy_executor::task]
pub async fn mock_dual_spin_estimator_task(
    state0: &'static SharedSpinState,
    state1: &'static SharedSpinState,
) -> ! {
    let mock0 = MockSpinEstimator::new(MOCK_SPIN_RATE);
    let mock1 = MockSpinEstimator::new(MOCK_SPIN_RATE);
    loop {
        Timer::after(EmbassyDuration::from_millis(1)).await;

        if CORE1_FLASH_PAUSE_REQUESTED.load(Ordering::Acquire) {
            info!("spin:mock paused for flash write");
            spin_estimator_pause_spin();
            info!("spin:mock resumed after flash write");
            continue;
        }

        let s0 = mock0.spin_state();
        state0.lock(|s| *s.borrow_mut() = s0);
        let mut s1 = mock1.spin_state();
        s1.position = (s1.position + MOCK_STRIP1_PHASE_OFFSET).constrain_circle();
        state1.lock(|s| *s.borrow_mut() = s1);
    }
}

#[cfg(feature = "imu-spin")]
const L3GD20H_ADDR: u8 = 0x6B;
#[cfg(feature = "imu-spin")]
const LSM303_ACCEL_ADDR: u8 = 0x19;
#[cfg(feature = "imu-spin")]
const LSM303_MAG_ADDR: u8 = 0x1E;

#[cfg(feature = "imu-spin")]
struct ImuSample {
    gyro_dps: nalgebra::Vector3<f32>,
    accel_g: nalgebra::Vector3<f32>,
    mag_gauss: nalgebra::Vector3<f32>,
}

#[cfg(feature = "imu-spin")]
async fn write_reg<I2C>(i2c: &mut I2C, addr: u8, reg: u8, value: u8) -> Result<(), I2C::Error>
where
    I2C: embedded_hal_async::i2c::I2c,
{
    i2c.write(addr, &[reg, value]).await
}

#[cfg(feature = "imu-spin")]
async fn read_regs<I2C>(i2c: &mut I2C, addr: u8, reg: u8, out: &mut [u8]) -> Result<(), I2C::Error>
where
    I2C: embedded_hal_async::i2c::I2c,
{
    i2c.write_read(addr, &[reg], out).await
}

#[cfg(feature = "imu-spin")]
async fn init_l3gd20h<I2C>(i2c: &mut I2C) -> Result<(), I2C::Error>
where
    I2C: embedded_hal_async::i2c::I2c,
{
    // CTRL_REG1: normal mode, all axes enabled, 760 Hz ODR.
    write_reg(i2c, L3GD20H_ADDR, 0x20, 0xEF).await?;
    // CTRL_REG4: full-scale 2000 dps (FS[1:0] = 11).
    write_reg(i2c, L3GD20H_ADDR, 0x23, 0x30).await
}

#[cfg(feature = "imu-spin")]
async fn init_lsm303<I2C>(i2c: &mut I2C) -> Result<(), I2C::Error>
where
    I2C: embedded_hal_async::i2c::I2c,
{
    // Accelerometer CTRL_REG1_A: 400 Hz, XYZ enabled.
    write_reg(i2c, LSM303_ACCEL_ADDR, 0x20, 0x77).await?;
    // Accelerometer CTRL_REG4_A: high-resolution, +/-2g.
    write_reg(i2c, LSM303_ACCEL_ADDR, 0x23, 0x08).await?;

    // Magnetometer: 15 Hz, +/-1.3 gauss, continuous-conversion mode.
    write_reg(i2c, LSM303_MAG_ADDR, 0x00, 0x10).await?;
    write_reg(i2c, LSM303_MAG_ADDR, 0x01, 0x20).await?;
    write_reg(i2c, LSM303_MAG_ADDR, 0x02, 0x00).await
}

#[cfg(feature = "imu-spin")]
async fn read_imu_sample<I2C>(i2c: &mut I2C) -> Result<ImuSample, I2C::Error>
where
    I2C: embedded_hal_async::i2c::I2c,
{
    const L3GD20H_SENSITIVITY_DPS_PER_LSB_2000DPS: f32 = 0.07;
    const LSM303_ACCEL_G_PER_LSB: f32 = 0.001;
    const LSM303_MAG_GAUSS_PER_LSB_1P3: f32 = 1.0 / 1100.0;

    let mut gyro_raw = [0u8; 6];
    let mut accel_raw = [0u8; 6];
    let mut mag_raw = [0u8; 6];

    // L3GD20H OUT_X_L with auto-increment.
    read_regs(i2c, L3GD20H_ADDR, 0x28 | 0x80, &mut gyro_raw).await?;
    // LSM303 accel OUT_X_L_A with auto-increment.
    read_regs(i2c, LSM303_ACCEL_ADDR, 0x28 | 0x80, &mut accel_raw).await?;
    // LSM303 mag OUT_X_H_M with auto-increment.
    read_regs(i2c, LSM303_MAG_ADDR, 0x03 | 0x80, &mut mag_raw).await?;

    let gx = i16::from_le_bytes([gyro_raw[0], gyro_raw[1]]) as f32;
    let gy = i16::from_le_bytes([gyro_raw[2], gyro_raw[3]]) as f32;
    let gz = i16::from_le_bytes([gyro_raw[4], gyro_raw[5]]) as f32;

    // LSM303 accel is left-aligned 12-bit in little-endian words.
    let ax = (i16::from_le_bytes([accel_raw[0], accel_raw[1]]) >> 4) as f32;
    let ay = (i16::from_le_bytes([accel_raw[2], accel_raw[3]]) >> 4) as f32;
    let az = (i16::from_le_bytes([accel_raw[4], accel_raw[5]]) >> 4) as f32;

    let gyro = nalgebra::Vector3::new(
        gx * L3GD20H_SENSITIVITY_DPS_PER_LSB_2000DPS,
        gy * L3GD20H_SENSITIVITY_DPS_PER_LSB_2000DPS,
        gz * L3GD20H_SENSITIVITY_DPS_PER_LSB_2000DPS,
    );

    let accel = nalgebra::Vector3::new(
        ax * LSM303_ACCEL_G_PER_LSB,
        ay * LSM303_ACCEL_G_PER_LSB,
        az * LSM303_ACCEL_G_PER_LSB,
    );

    let mx = i16::from_be_bytes([mag_raw[0], mag_raw[1]]) as f32;
    let mz = i16::from_be_bytes([mag_raw[2], mag_raw[3]]) as f32;
    let my = i16::from_be_bytes([mag_raw[4], mag_raw[5]]) as f32;

    let mag_gauss = nalgebra::Vector3::new(
        mx * LSM303_MAG_GAUSS_PER_LSB_1P3,
        my * LSM303_MAG_GAUSS_PER_LSB_1P3,
        mz * LSM303_MAG_GAUSS_PER_LSB_1P3,
    );

    Ok(ImuSample {
        gyro_dps: gyro,
        accel_g: accel,
        mag_gauss,
    })
}

#[cfg(feature = "imu-spin")]
struct CalibrationData {
    gyro_bias_dps: nalgebra::Vector3<f32>,
    calibrating_gyro_bias: bool,
    calibration_accum_dps: nalgebra::Vector3<f32>,
    calibration_elapsed_s: f32,
    calibration_samples: u32,
    calibration_reset_log_divider: u8,
}

#[cfg(feature = "imu-spin")]
struct SampleRateMonitor {
    sample_counter: u32,
    sample_time_accum_s: f32,
}

#[cfg(feature = "imu-spin")]
fn check_sample_rate(monitor: &mut SampleRateMonitor, dt: f32) {
    monitor.sample_counter = monitor.sample_counter.wrapping_add(1);
    monitor.sample_time_accum_s += dt;
    if monitor.sample_counter >= 500 {
        let hz = monitor.sample_counter as f32 / monitor.sample_time_accum_s.max(1e-6);
        if hz < 100.0 {
            defmt::warn!("spin:imu low sample rate hz={=f32}", hz);
        } else {
            info!("spin:imu sample rate hz={=f32}", hz);
        }
        monitor.sample_counter = 0;
        monitor.sample_time_accum_s = 0.0;
    }
}

#[cfg(feature = "imu-spin")]
fn check_and_initialize_gyro_bias(
    calibration_data: &mut CalibrationData,
    sample: &ImuSample,
    dt: f32,
    last_angle: Angle,
    state0: &SharedSpinState,
    state1: &SharedSpinState,
) -> bool {
    #[cfg(feature = "imu-spin")]
    const IMU_CALIBRATION_DURATION_S: f32 = 5.0;
    #[cfg(feature = "imu-spin")]
    const IMU_CALIBRATION_MOTION_MAX_DPS: f32 = 100.0;

    if !calibration_data.calibrating_gyro_bias {
        return false;
    }

    let gyro_norm_dps = sample.gyro_dps.norm();
    if gyro_norm_dps <= IMU_CALIBRATION_MOTION_MAX_DPS {
        calibration_data.calibration_accum_dps += sample.gyro_dps;
        calibration_data.calibration_elapsed_s += dt;
        calibration_data.calibration_samples = calibration_data.calibration_samples.wrapping_add(1);

        if calibration_data.calibration_elapsed_s >= IMU_CALIBRATION_DURATION_S
            && calibration_data.calibration_samples > 0
        {
            let inv_n = 1.0 / calibration_data.calibration_samples as f32;
            calibration_data.gyro_bias_dps = calibration_data.calibration_accum_dps * inv_n;
            calibration_data.calibrating_gyro_bias = false;
            info!(
                "spin:imu gyro bias calibrated dps=({=f32}, {=f32}, {=f32})",
                calibration_data.gyro_bias_dps.x,
                calibration_data.gyro_bias_dps.y,
                calibration_data.gyro_bias_dps.z,
            );
        }
    } else {
        calibration_data.calibration_accum_dps = nalgebra::Vector3::new(0.0f32, 0.0, 0.0);
        calibration_data.calibration_elapsed_s = 0.0;
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
        *s.borrow_mut() = SpinState {
            position: last_angle,
            rate: zero_rate,
        };
    });
    state1.lock(|s| {
        *s.borrow_mut() = SpinState {
            position: last_angle,
            rate: zero_rate,
        };
    });

    true
}

/// IMU-based dual-strip spin estimator using L3GD20H + LSM303 and fusion-ahrs.
///
/// Polls both sensors over I2C and fuses all three (gyroscope, accelerometer,
/// magnetometer) via the Fusion AHRS complementary filter into a drift-corrected
/// `UnitQuaternion`. The quaternion is used directly to compute the spoke wheel
/// angle each frame. Once the AHRS has converged and the wheel is spinning, a
/// body-fixed reference direction (world-up projected into the spin plane in body
/// frame) is captured once; subsequent angles are measured relative to that
/// reference so that zero degrees corresponds to the world-up direction.
#[cfg(feature = "imu-spin")]
#[embassy_executor::task]
pub async fn imu_dual_spin_estimator_task(
    state0: &'static SharedSpinState,
    state1: &'static SharedSpinState,
    mut i2c: esp_hal::i2c::master::I2c<'static, esp_hal::Async>,
) -> ! {
    use fusion_ahrs::{Ahrs, AhrsSettings, Convention};

    const GYRO_AXIS_MIN_RATE_DPS: f32 = 30.0;
    const GRAVITY_PROJECTION_MIN_NORM: f32 = 0.2;
    const IMU_ANGLE_DIRECTION: f32 = -1.0;
    const STRIP0_PHASE_OFFSET_FROM_SENSOR: Angle = Angle::from_degrees(90.0);
    const STRIP1_PHASE_OFFSET_FROM_SENSOR: Angle = Angle::from_degrees(-90.0);
    const OFFSET_CALIBRATION: Angle = Angle::from_degrees(-75.0);

    let settings = AhrsSettings {
        convention: Convention::Nwu,
        gain: 0.50,
        gyroscope_range: 2000.0,
        acceleration_rejection: 15.0,
        recovery_trigger_period: 1000,
        magnetic_rejection: 15.0,
    };

    let mut ahrs = Ahrs::with_settings(settings);
    let mut last = Instant::now();
    let mut error_log_divider: u8 = 0;
    let mut initialized = false;
    let mut last_angle = Angle::from_radians(0.0);
    let mut sample_rate_monitor = SampleRateMonitor {
        sample_counter: 0,
        sample_time_accum_s: 0.0,
    };

    let mut calibration_data = CalibrationData {
        gyro_bias_dps: nalgebra::Vector3::new(0.0f32, 0.0, 0.0),
        calibrating_gyro_bias: true,
        calibration_accum_dps: nalgebra::Vector3::new(0.0f32, 0.0, 0.0),
        calibration_elapsed_s: 0.0,
        calibration_samples: 0,
        calibration_reset_log_divider: 0,
    };

    // Body-frame reference direction captured once after AHRS convergence.
    // Equals world-up projected onto the spin plane in body frame, so the
    // output angle is 0 when that direction points toward world-up.
    let mut ref_body: Option<nalgebra::Vector3<f32>> = None;

    loop {
        Timer::after(EmbassyDuration::from_millis(1)).await;

        if CORE1_FLASH_PAUSE_REQUESTED.load(Ordering::Acquire) {
            info!("spin:imu paused for flash write");
            spin_estimator_pause_spin();
            info!("spin:imu resumed after flash write");
            continue;
        }

        let now = Instant::now();
        let dt = now.duration_since(last).as_micros() as f32 * 1e-6;
        last = now;

        if !initialized {
            if init_l3gd20h(&mut i2c).await.is_err() || init_lsm303(&mut i2c).await.is_err() {
                error_log_divider = error_log_divider.wrapping_add(1);
                if error_log_divider == 0 {
                    defmt::warn!("spin:imu sensor init failed; retrying");
                }
                Timer::after(EmbassyDuration::from_millis(100)).await;
                continue;
            }
            initialized = true;
            info!("spin:imu sensors initialized");
        }

        match read_imu_sample(&mut i2c).await {
            Ok(sample) => {
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
                ahrs.update(corrected_gyro_dps, sample.accel_g, sample.mag_gauss, dt);

                check_sample_rate(&mut sample_rate_monitor, dt);

                let gyro_rate_dps = corrected_gyro_dps.norm();
                let signed_rate_dps = IMU_ANGLE_DIRECTION * gyro_rate_dps;
                let rate = AngularVelocity::from_degrees_secs(signed_rate_dps);

                let q = ahrs.quaternion();
                // NWU convention: world Z = up.
                let world_up = nalgebra::Vector3::new(0.0f32, 0.0, 1.0);

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
                        //   up_in_spin_frame  — direction of world-up within the plane
                        //   e2    — 90° ahead in the direction of rotation
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
                        }
                    }
                }

                let strip0_angle =
                    (last_angle + STRIP0_PHASE_OFFSET_FROM_SENSOR + OFFSET_CALIBRATION)
                        .constrain_circle();

                let strip1_angle =
                    (last_angle + STRIP1_PHASE_OFFSET_FROM_SENSOR + OFFSET_CALIBRATION)
                        .constrain_circle();

                state0.lock(|s| {
                    *s.borrow_mut() = SpinState {
                        position: strip0_angle,
                        rate,
                    };
                });
                state1.lock(|s| {
                    *s.borrow_mut() = SpinState {
                        position: strip1_angle,
                        rate,
                    };
                });
            }
            Err(_) => {
                error_log_divider = error_log_divider.wrapping_add(1);
                if error_log_divider == 0 {
                    defmt::warn!("spin:imu sensor read failed");
                }
            }
        }
    }
}
