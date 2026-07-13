use bmi2::config;
use bmi2::interface::I2cInterface;
use bmi2::types::{BMI260_CHIP_ID, BMI270_CHIP_ID, Burst, Error as Bmi2Error, PwrCtrl};
use bmi2::{Bmi2, I2cAddr};
use core::sync::atomic::Ordering;
use core::time::Duration;
use defmt::info;
use embassy_time::{Delay, Duration as EmbassyDuration, Instant, Timer};
#[cfg(feature = "hybrid-angle-estimator")]
use pov_algs::AngularVelocity;

const BMI260_ACCEL_G_PER_LSB_2G: f32 = 1.0 / 16384.0;
const BMI260_GYRO_DPS_PER_LSB_2000DPS: f32 = 1.0 / 16.4;
const IMU_SAMPLE_RATE_WARNING_THRESHOLD: EmbassyDuration = EmbassyDuration::from_hz(5);

struct SampleRateMonitor {
    sample_counter: u32,
    sample_time_accum: Duration,
}

type Bmi2Device<'a> = Bmi2<I2cInterface<&'a mut super::SharedI2cDevice>, Delay>;

fn i2c_addr_u8(address: I2cAddr) -> u8 {
    match address {
        I2cAddr::Default => 0x68,
        I2cAddr::Alternative => 0x69,
    }
}

async fn try_init_bmi2_on_addr<'a>(
    i2c: &'a mut super::SharedI2cDevice,
    address: I2cAddr,
) -> Option<(Bmi2Device<'a>, u8)> {
    let addr_u8 = i2c_addr_u8(address);
    let mut bmi = Bmi2::new_i2c(i2c, Delay, address, Burst::new(255));

    let chip_id = match bmi.get_chip_id().await {
        Ok(id) => id,
        Err(_) => return None,
    };

    if chip_id != BMI260_CHIP_ID && chip_id != BMI270_CHIP_ID {
        defmt::warn!(
            "imu:bmi260 unknown chip id=0x{=u8:02x} addr=0x{=u8:02x}",
            chip_id,
            addr_u8
        );
        return None;
    }

    let mut config_buf = [0u8; 256];
    if bmi
        .init(&config::BMI260_CONFIG_FILE, &mut config_buf)
        .await
        .is_err()
    {
        defmt::warn!("imu:bmi260 init failed addr=0x{=u8:02x}", addr_u8);
        return None;
    }

    if bmi
        .set_pwr_ctrl(PwrCtrl {
            aux_en: false,
            gyr_en: true,
            acc_en: true,
            temp_en: true,
        })
        .await
        .is_err()
    {
        defmt::warn!("imu:bmi260 set pwr-ctrl failed addr=0x{=u8:02x}", addr_u8);
        return None;
    }

    info!(
        "imu:bmi260 initialized chip=0x{=u8:02x} addr=0x{=u8:02x}",
        chip_id, addr_u8
    );

    Some((bmi, chip_id))
}

async fn read_imu_sample<I2C>(
    bmi: &mut Bmi2<I2cInterface<I2C>, Delay>,
) -> Result<super::ImuSample, Bmi2Error<I2C::Error>>
where
    I2C: embedded_hal_async::i2c::I2c,
{
    let data = bmi.get_data().await?;

    let ax = data.acc.x as f32;
    let ay = data.acc.y as f32;
    let az = data.acc.z as f32;
    let gx = data.gyr.x as f32;
    let gy = data.gyr.y as f32;
    let gz = data.gyr.z as f32;

    Ok(super::ImuSample {
        gyro_dps: nalgebra::Vector3::new(
            gx * BMI260_GYRO_DPS_PER_LSB_2000DPS,
            gy * BMI260_GYRO_DPS_PER_LSB_2000DPS,
            gz * BMI260_GYRO_DPS_PER_LSB_2000DPS,
        ),
        accel_g: nalgebra::Vector3::new(
            ax * BMI260_ACCEL_G_PER_LSB_2G,
            ay * BMI260_ACCEL_G_PER_LSB_2G,
            az * BMI260_ACCEL_G_PER_LSB_2G,
        ),
    })
}

fn check_sample_rate(monitor: &mut SampleRateMonitor, dt: Duration) {
    monitor.sample_counter = monitor.sample_counter.wrapping_add(1);
    monitor.sample_time_accum += dt;
    if monitor.sample_counter >= 500 {
        let elapsed_s = monitor.sample_time_accum.as_secs_f32().max(1e-6);
        let hz = (monitor.sample_counter as f32 / elapsed_s) as u64;
        let min_hz = 1000 / IMU_SAMPLE_RATE_WARNING_THRESHOLD.as_millis();

        if hz < min_hz {
            defmt::warn!("imu:bmi260 low sample rate hz={=u64}", hz);
        } else {
            defmt::debug!("imu:bmi260 sample rate hz={=u64}", hz);
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

#[esp_hal::ram]
fn imu_pause_spin() {
    crate::led::CORE1_FLASH_PAUSED_COUNT.fetch_add(1, Ordering::Release);
    while crate::led::CORE1_FLASH_PAUSE_REQUESTED.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }
    crate::led::CORE1_FLASH_PAUSED_COUNT.fetch_sub(1, Ordering::Release);
}

fn pause_needed_for_flash() -> bool {
    crate::led::CORE1_FLASH_PAUSE_REQUESTED.load(Ordering::Acquire)
}

pub async fn imu_publisher_impl(mut i2c: super::SharedI2cDevice) -> ! {
    let mut error_log_divider: u8 = 0;
    let mut sample_rate_monitor = SampleRateMonitor {
        sample_counter: 0,
        sample_time_accum: Duration::ZERO,
    };

    loop {
        let mut bmi = loop {
            if let Some((dev, _chip_id)) = try_init_bmi2_on_addr(&mut i2c, I2cAddr::Default).await {
                break dev;
            }
            if let Some((dev, _chip_id)) =
                try_init_bmi2_on_addr(&mut i2c, I2cAddr::Alternative).await
            {
                break dev;
            }

            error_log_divider = error_log_divider.wrapping_add(1);
            if error_log_divider == 0 {
                defmt::warn!("imu:bmi260 init failed; no device detected; retrying");
            }
            Timer::after(EmbassyDuration::from_millis(100)).await;
        };

        let mut last = Instant::now();

        loop {
            Timer::after(EmbassyDuration::from_millis(1)).await;

            if pause_needed_for_flash() {
                info!("imu:bmi260 paused for flash write");
                imu_pause_spin();
                info!("imu:bmi260 resumed after flash write");
                continue;
            }

            let now = Instant::now();
            let dt = Duration::from_micros(now.duration_since(last).as_micros());
            last = now;

            match read_imu_sample(&mut bmi).await {
                Ok(sample) => {
                    #[cfg(feature = "hybrid-angle-estimator")]
                    {
                        super::publish_spin_rate(AngularVelocity::from_degrees_secs(
                            dominant_signed_rate_dps(sample.gyro_dps),
                        ));
                    }
                    super::publish_sample(sample);
                    check_sample_rate(&mut sample_rate_monitor, dt);
                }
                Err(_) => {
                    error_log_divider = error_log_divider.wrapping_add(1);
                    if error_log_divider == 0 {
                        defmt::warn!("imu:bmi260 sample read failed; reinitializing");
                    }
                    break;
                }
            }
        }
    }
}
