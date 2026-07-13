mod bmi260;

#[cfg(feature = "hybrid-angle-estimator")]
use pov_algs::AngularVelocity;

type SharedI2cDevice = embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice<
    'static,
    embassy_sync::blocking_mutex::raw::NoopRawMutex,
    esp_hal::i2c::master::I2c<'static, esp_hal::Async>,
>;

const IMU_SAMPLE_CAPACITY: usize = 16;
const IMU_SAMPLE_SUBSCRIBERS: usize = 4;
const IMU_SAMPLE_PUBLISHERS: usize = 1;

#[cfg(feature = "hybrid-angle-estimator")]
const IMU_SPIN_RATE_CAPACITY: usize = 16;
#[cfg(feature = "hybrid-angle-estimator")]
const IMU_SPIN_RATE_SUBSCRIBERS: usize = 2;
#[cfg(feature = "hybrid-angle-estimator")]
const IMU_SPIN_RATE_PUBLISHERS: usize = 1;

#[derive(Clone, Copy)]
pub struct ImuSample {
    pub gyro_dps: nalgebra::Vector3<f32>,
    pub accel_g: nalgebra::Vector3<f32>,
}

pub type ImuSampleSubscriber = embassy_sync::pubsub::Subscriber<
    'static,
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    ImuSample,
    IMU_SAMPLE_CAPACITY,
    IMU_SAMPLE_SUBSCRIBERS,
    IMU_SAMPLE_PUBLISHERS,
>;

static IMU_SAMPLES: embassy_sync::pubsub::PubSubChannel<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    ImuSample,
    IMU_SAMPLE_CAPACITY,
    IMU_SAMPLE_SUBSCRIBERS,
    IMU_SAMPLE_PUBLISHERS,
> = embassy_sync::pubsub::PubSubChannel::new();

#[cfg(feature = "hybrid-angle-estimator")]
pub type SpinRateSubscriber = embassy_sync::pubsub::Subscriber<
    'static,
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    AngularVelocity,
    IMU_SPIN_RATE_CAPACITY,
    IMU_SPIN_RATE_SUBSCRIBERS,
    IMU_SPIN_RATE_PUBLISHERS,
>;

#[cfg(feature = "hybrid-angle-estimator")]
static IMU_SPIN_RATE_SAMPLES: embassy_sync::pubsub::PubSubChannel<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    AngularVelocity,
    IMU_SPIN_RATE_CAPACITY,
    IMU_SPIN_RATE_SUBSCRIBERS,
    IMU_SPIN_RATE_PUBLISHERS,
> = embassy_sync::pubsub::PubSubChannel::new();

pub fn subscribe() -> Option<ImuSampleSubscriber> {
    IMU_SAMPLES.subscriber().ok()
}

#[cfg(feature = "hybrid-angle-estimator")]
pub fn subscribe_spin_rate() -> Option<SpinRateSubscriber> {
    IMU_SPIN_RATE_SAMPLES.subscriber().ok()
}

fn publish_sample(sample: ImuSample) {
    IMU_SAMPLES.immediate_publisher().publish_immediate(sample);
}

#[cfg(feature = "hybrid-angle-estimator")]
fn publish_spin_rate(rate: AngularVelocity) {
    IMU_SPIN_RATE_SAMPLES
        .immediate_publisher()
        .publish_immediate(rate);
}

#[embassy_executor::task]
pub async fn imu_publisher_task(i2c: SharedI2cDevice) -> ! {
    bmi260::imu_publisher_impl(i2c).await
}
