#![allow(dead_code)]

use core::cell::RefCell;
use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use core::task::{Context, Poll, Waker};

use critical_section::Mutex;
use esp_hal::analog::adc::{AdcChannel, AdcPin, Attenuation};
use esp_hal::handler;
use esp_hal::interrupt::Priority;
use esp_hal::peripherals;
use esp_hal::peripherals::{ADC1, APB_SARADC, Interrupt, SENS, SYSTEM};

const ADC_DIGITAL_RAW_MASK: u32 = 0x0fff;

// ---- Compile-time channel range guarantee ----
// The APB_SARADC threshold monitor hardware can only address ADC1 channels
// 0–7 (ESP32-S3: GPIO1–GPIO8). Channels 8 and 9 (GPIO9, GPIO10) are ADC1
// pins but are NOT reachable by the monitor comparators.
//
// `Adc1MonitorChannel` is a sealed trait; only the eight compatible GPIO
// types implement it, so passing GPIO9/GPIO10 or any ADC2 pin is a
// compile-time error.
mod sealed {
    pub trait Sealed {}
}

/// Marker trait for ADC1 pins addressable by the digital monitor hardware
/// (channels 0–7, i.e. GPIO1–GPIO8 on ESP32-S3).
pub trait Adc1MonitorChannel: sealed::Sealed + AdcChannel {}

macro_rules! impl_adc1_monitor_channel {
    ($($gpio:ident),+ $(,)?) => {
        $(
            impl sealed::Sealed for peripherals::$gpio<'_> {}
            impl Adc1MonitorChannel for peripherals::$gpio<'_> {}
        )+
    };
}

impl_adc1_monitor_channel!(GPIO1, GPIO2, GPIO3, GPIO4, GPIO5, GPIO6, GPIO7, GPIO8);

// ---- Monitor 0 state ----
static ADC_MONITOR_TRIGGERED: AtomicBool = AtomicBool::new(false);
static ADC_MONITOR_EVENT: AtomicU8 = AtomicU8::new(0);
static ADC_MONITOR_LAST_SAMPLE: AtomicU32 = AtomicU32::new(0);
static ADC_MONITOR_WAKER: Mutex<RefCell<Option<Waker>>> = Mutex::new(RefCell::new(None));

// ---- Monitor 1 state ----
static ADC_MONITOR1_TRIGGERED: AtomicBool = AtomicBool::new(false);
static ADC_MONITOR1_EVENT: AtomicU8 = AtomicU8::new(0);
static ADC_MONITOR1_LAST_SAMPLE: AtomicU32 = AtomicU32::new(0);
static ADC_MONITOR1_WAKER: Mutex<RefCell<Option<Waker>>> = Mutex::new(RefCell::new(None));

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThresholdEvent {
    High,
    Low,
    Both,
}

impl ThresholdEvent {
    fn as_u8(self) -> u8 {
        match self {
            Self::High => 1,
            Self::Low => 2,
            Self::Both => 3,
        }
    }

    fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::High),
            2 => Some(Self::Low),
            3 => Some(Self::Both),
            _ => None,
        }
    }
}

/// Uncalibrated thresholds are 12-bit raw values (0–4095) corresponding to the full ADC range.
/// The actual voltage thresholds depend on the attenuation setting and the input voltage.
#[derive(Clone, Copy, Debug)]
pub struct MonitorThreshold {
    pub low: u16,
    pub high: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct SampleRate {
    pub timer_target: u16,
    pub sar_clk_div: u8,
}

impl Default for SampleRate {
    fn default() -> Self {
        Self {
            // Conservative defaults close to ESP-IDF behavior for periodic digital sampling.
            timer_target: 200,
            sar_clk_div: 4,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MonitorConfig {
    pub attenuation: Attenuation,
    pub threshold: MonitorThreshold,
    pub sample_rate: SampleRate,
}

// ---- Compile-time mode markers ----

/// Use only monitor 0 (threshold comparator 0). Default mode.
pub struct Single;

/// Use both monitor 0 and monitor 1 (threshold comparators 0 and 1).
pub struct Dual;

// ---- Compile-time sample-tracking markers ----

/// Enables the `last_sample()` methods on the monitor (default).
pub struct TrackSample;

/// Disables the `last_sample()` API; the monitor only fires threshold events.
pub struct NoSample;

/// Sealed compile-time trait implemented by [`TrackSample`] and [`NoSample`].
pub trait SampleTracking: sealed::Sealed {
    /// `true` when this marker enables last-sample storage in the interrupt handler.
    const ENABLED: bool;
}

impl sealed::Sealed for TrackSample {}
impl SampleTracking for TrackSample {
    const ENABLED: bool = true;
}

impl sealed::Sealed for NoSample {}
impl SampleTracking for NoSample {
    const ENABLED: bool = false;
}

/// ADC used in continuous monitoring mode with interrupt-on-threshold
/// functionality. Only ADC1 is supported for the ESP32-S3's digital ADC.
/// ADC2's digital mode is broken.
pub struct AdcMonitor<'d, PIN0, PIN1 = (), Mode = Single, S = TrackSample> {
    _adc1: ADC1<'d>,
    _pin0: PIN0,
    _pin1: PIN1,
    monitor_channel: u8,
    monitor1_channel: u8, // only meaningful in Dual mode
    _mode: PhantomData<Mode>,
    _sample: PhantomData<S>,
}

// ---- Interrupt handlers ----
// Two variants are provided so the constructor can bind the appropriate one
// at initialisation time, avoiding any runtime branching in the hot path.

/// ISR variant used when at least one monitor has sample tracking enabled.
/// Reads the ADC data register and stores the raw value alongside the event.
#[handler(priority = Priority::Priority2)]
fn adc_monitor_interrupt_handler_tracking() {
    let saradc = APB_SARADC::regs();
    let status = saradc.int_st().read();

    let high0 = status.thres0_high().bit_is_set();
    let low0 = status.thres0_low().bit_is_set();
    let high1 = status.thres1_high().bit_is_set();
    let low1 = status.thres1_low().bit_is_set();

    if !(high0 || low0 || high1 || low1) {
        return;
    }

    let raw = saradc
        .apb_saradc1_data_status()
        .read()
        .saradc1_data()
        .bits()
        & ADC_DIGITAL_RAW_MASK;

    if high0 || low0 {
        let event = if high0 && low0 {
            ThresholdEvent::Both
        } else if high0 {
            ThresholdEvent::High
        } else {
            ThresholdEvent::Low
        };

        ADC_MONITOR_LAST_SAMPLE.store(raw, Ordering::Release);
        ADC_MONITOR_EVENT.store(event.as_u8(), Ordering::Release);
        ADC_MONITOR_TRIGGERED.store(true, Ordering::Release);

        critical_section::with(|cs| {
            if let Some(waker) = ADC_MONITOR_WAKER.borrow(cs).borrow().as_ref() {
                waker.wake_by_ref();
            }
        });
    }

    if high1 || low1 {
        let event = if high1 && low1 {
            ThresholdEvent::Both
        } else if high1 {
            ThresholdEvent::High
        } else {
            ThresholdEvent::Low
        };

        ADC_MONITOR1_LAST_SAMPLE.store(raw, Ordering::Release);
        ADC_MONITOR1_EVENT.store(event.as_u8(), Ordering::Release);
        ADC_MONITOR1_TRIGGERED.store(true, Ordering::Release);

        critical_section::with(|cs| {
            if let Some(waker) = ADC_MONITOR1_WAKER.borrow(cs).borrow().as_ref() {
                waker.wake_by_ref();
            }
        });
    }

    saradc.int_clr().write(|w| {
        if high0 {
            w.thres0_high().clear_bit_by_one();
        }
        if low0 {
            w.thres0_low().clear_bit_by_one();
        }
        if high1 {
            w.thres1_high().clear_bit_by_one();
        }
        if low1 {
            w.thres1_low().clear_bit_by_one();
        }
        w
    });
}

/// ISR variant used when sample tracking is disabled for all monitors.
/// Skips the ADC data register read entirely.
#[handler(priority = Priority::Priority2)]
fn adc_monitor_interrupt_handler_no_sample() {
    let saradc = APB_SARADC::regs();
    let status = saradc.int_st().read();

    let high0 = status.thres0_high().bit_is_set();
    let low0 = status.thres0_low().bit_is_set();
    let high1 = status.thres1_high().bit_is_set();
    let low1 = status.thres1_low().bit_is_set();

    if !(high0 || low0 || high1 || low1) {
        return;
    }

    if high0 || low0 {
        let event = if high0 && low0 {
            ThresholdEvent::Both
        } else if high0 {
            ThresholdEvent::High
        } else {
            ThresholdEvent::Low
        };

        ADC_MONITOR_EVENT.store(event.as_u8(), Ordering::Release);
        ADC_MONITOR_TRIGGERED.store(true, Ordering::Release);

        critical_section::with(|cs| {
            if let Some(waker) = ADC_MONITOR_WAKER.borrow(cs).borrow().as_ref() {
                waker.wake_by_ref();
            }
        });
    }

    if high1 || low1 {
        let event = if high1 && low1 {
            ThresholdEvent::Both
        } else if high1 {
            ThresholdEvent::High
        } else {
            ThresholdEvent::Low
        };

        ADC_MONITOR1_EVENT.store(event.as_u8(), Ordering::Release);
        ADC_MONITOR1_TRIGGERED.store(true, Ordering::Release);

        critical_section::with(|cs| {
            if let Some(waker) = ADC_MONITOR1_WAKER.borrow(cs).borrow().as_ref() {
                waker.wake_by_ref();
            }
        });
    }

    saradc.int_clr().write(|w| {
        if high0 {
            w.thres0_high().clear_bit_by_one();
        }
        if low0 {
            w.thres0_low().clear_bit_by_one();
        }
        if high1 {
            w.thres1_high().clear_bit_by_one();
        }
        if low1 {
            w.thres1_low().clear_bit_by_one();
        }
        w
    });
}

// ---- Methods shared by both Single and Dual modes ----
impl<'d, PIN0, PIN1, Mode, S> AdcMonitor<'d, PIN0, PIN1, Mode, S> {
    pub fn start(&mut self) {
        let saradc = APB_SARADC::regs();
        // Reset the digital controller FSM before starting.
        saradc.dma_conf().modify(|_, w| w.adc_reset_fsm().set_bit());
        saradc
            .dma_conf()
            .modify(|_, w| w.adc_reset_fsm().clear_bit());
        // Enable the periodic timer to begin conversions.
        saradc.ctrl2().modify(|_, w| w.timer_en().set_bit());
    }

    pub fn stop(&mut self) {
        APB_SARADC::regs()
            .ctrl2()
            .modify(|_, w| w.timer_en().clear_bit());
        APB_SARADC::regs()
            .ctrl()
            .modify(|_, w| w.start().clear_bit());
    }
}

// ---- Shared ADC initialisation helpers ----

/// Phase 0–3: bus clock + reset, analog clock/power, ADC1 mux routing,
/// FSM timing defaults, and APB clock source.  Called by both `new` and
/// `new_dual` before any channel-specific setup.
fn init_apb_saradc_clock_and_power() {
    let system = SYSTEM::regs();
    let sens = SENS::regs();
    let saradc = APB_SARADC::regs();

    // Phase 0: Enable APB_SARADC bus clock and pulse-reset the peripheral.
    system
        .perip_clk_en0()
        .modify(|_, w| w.apb_saradc_clk_en().set_bit());
    system
        .perip_rst_en0()
        .modify(|_, w| w.apb_saradc_rst().set_bit());
    system
        .perip_rst_en0()
        .modify(|_, w| w.apb_saradc_rst().clear_bit());

    // Phase 1: Analog clock + power.
    sens.sar_peri_clk_gate_conf()
        .modify(|_, w| w.saradc_clk_en().set_bit());
    // SAFETY: force_xpd_sar is a 2-bit field; 0b11 is the documented value
    // that keeps the SAR ADC powered during digital conversions.
    sens.sar_power_xpd_sar()
        .modify(|_, w| unsafe { w.force_xpd_sar().bits(0b11) });

    // Route ADC1 to the digital controller.
    sens.sar_meas1_mux()
        .modify(|_, w| w.sar1_dig_force().set_bit());
    sens.sar_meas1_ctrl2().modify(|_, w| {
        w.meas1_start_force().set_bit();
        w.sar1_en_pad_force().set_bit()
    });

    // Phase 2: FSM timing defaults (match ESP-IDF).
    // SAFETY: rstb_wait, xpd_wait, and standby_wait are timing counters with
    // no invalid bit patterns; values are conservative defaults from ESP-IDF.
    saradc.fsm_wait().write(|w| unsafe {
        w.rstb_wait().bits(8);
        w.xpd_wait().bits(5);
        w.standby_wait().bits(100)
    });

    // Phase 3: Clock source — APB (clk_sel=2), divider 16.
    // SAFETY: clkm_div_num/b/a are clock divider fields with no invalid
    // patterns; clk_sel=2 selects the APB clock source per the ESP32-S3 TRM.
    saradc.clkm_conf().write(|w| unsafe {
        w.clkm_div_num().bits(15);
        w.clkm_div_b().bits(1);
        w.clkm_div_a().bits(0);
        w.clk_sel().bits(2)
    });
}

/// Configure the attenuation for a single ADC1 channel in SAR_ATTEN1.
fn set_channel_attenuation(channel: u8, attenuation: u8) {
    SENS::regs().sar_atten1().modify(|r, w| {
        let shift = (channel as usize) * 2;
        let mask = !(0b11_u32 << shift);
        let value = (r.bits() & mask) | (((attenuation & 0b11) as u32) << shift);
        // SAFETY: The value is built by reading the existing register, clearing
        // the two bits for the target channel, and OR-ing in the validated 2-bit
        // attenuation, so no reserved bits are disturbed.
        unsafe { w.sar1_atten().bits(value) }
    });
}

/// Phase 4: digital controller `ctrl` / `ctrl2` registers.
/// `patt_len` is the zero-based pattern-table length field (0 = 1 entry, 1 = 2 entries).
fn configure_digital_controller(sample_rate: SampleRate, patt_len: u8) {
    let saradc = APB_SARADC::regs();
    // SAFETY: All multi-bit fields written here are validated configuration
    // constants from the ESP32-S3 TRM: work_mode=0 selects single-channel mode;
    // sar_clk_div and sar1_patt_len are hardware-range values supplied by the
    // caller; xpd_sar_force=0b11 is the documented value that keeps the SAR
    // powered during digital conversions.
    saradc.ctrl().modify(|_, w| unsafe {
        w.start_force().clear_bit();
        w.work_mode().bits(0);
        w.sar_sel().clear_bit();
        w.sar_clk_gated().set_bit();
        w.sar_clk_div().bits(sample_rate.sar_clk_div);
        w.sar1_patt_len().bits(patt_len);
        w.sar1_patt_p_clear().set_bit();
        w.data_sar_sel().set_bit();
        w.xpd_sar_force().bits(0b11)
    });
    saradc
        .ctrl()
        .modify(|_, w| w.sar1_patt_p_clear().clear_bit());
    // SAFETY: timer_target is a 12-bit hardware field; the value comes from
    // MonitorConfig, which callers are responsible for keeping in range.
    saradc.ctrl2().modify(|_, w| unsafe {
        w.meas_num_limit().clear_bit();
        w.timer_sel().set_bit();
        w.timer_target().bits(sample_rate.timer_target)
        // timer_en is deferred to start()
    });
}

/// Bind and enable the APB_ADC interrupt at Priority2 with the given handler.
fn bind_and_enable_interrupt(handler: esp_hal::interrupt::IsrCallback) {
    // SAFETY: `handler` is a valid ISR obtained from the `#[handler]` macro,
    // which guarantees the correct calling convention and interrupt-safe
    // alignment. The caller holds exclusive ownership of `ADC1<'d>`, so no
    // other code can concurrently reconfigure or rebind the APB_ADC vector.
    unsafe {
        esp_hal::interrupt::bind_interrupt(Interrupt::APB_ADC, handler);
    }
    esp_hal::interrupt::enable(Interrupt::APB_ADC, Priority::Priority2).unwrap();
}

// ---- Single-monitor mode (default) ----
impl<'d, PIN0, S: SampleTracking> AdcMonitor<'d, PIN0, (), Single, S>
where
    PIN0: Adc1MonitorChannel,
{
    fn new_impl<CS>(
        adc1: ADC1<'d>,
        pin: AdcPin<PIN0, ADC1<'d>, CS>,
        config: MonitorConfig,
    ) -> Self {
        let AdcPin { pin, .. } = pin;
        let channel = pin.adc_channel();
        let attenuation = config.attenuation as u8;
        let threshold_low = config.threshold.low.min(ADC_DIGITAL_RAW_MASK as u16);
        let threshold_high = config.threshold.high.min(ADC_DIGITAL_RAW_MASK as u16);

        let saradc = APB_SARADC::regs();

        init_apb_saradc_clock_and_power();
        set_channel_attenuation(channel, attenuation);

        // Pattern entry format: [channel(4b):atten(2b)] = 6 bits.
        // Entry 0 occupies bits [23:18] of the 24-bit sar1_patt_tab register.
        let patt_entry = ((((channel & 0x0f) as u32) << 2) | ((attenuation & 0x03) as u32)) << 18;
        // SAFETY: patt_entry packs a 4-bit channel (0–7, enforced by
        // Adc1MonitorChannel) and a 2-bit attenuation into bits [23:18] per the
        // ESP32-S3 TRM pattern-table format; all other bits are zero.
        saradc
            .sar1_patt_tab1()
            .write(|w| unsafe { w.sar1_patt_tab1().bits(patt_entry) });

        configure_digital_controller(config.sample_rate, 0); // 0 => 1 entry

        // SAFETY: channel is in 0–7 (enforced by Adc1MonitorChannel); threshold
        // values are clamped to ADC_DIGITAL_RAW_MASK (12 bits) above.
        // Program monitor 0 threshold for the requested ADC1 channel.
        saradc.thres0_ctrl().write(|w| unsafe {
            w.thres0_channel().bits(channel);
            w.thres0_high().bits(threshold_high);
            w.thres0_low().bits(threshold_low)
        });
        saradc.thres_ctrl().modify(|_, w| {
            w.thres_all_en().clear_bit();
            w.thres1_en().clear_bit();
            w.thres0_en().set_bit()
        });

        // Clear stale interrupts and enable monitor 0 IRQs only.
        saradc.int_clr().write(|w| {
            w.thres0_high().clear_bit_by_one();
            w.thres0_low().clear_bit_by_one();
            w
        });
        saradc.int_ena().modify(|_, w| {
            w.thres0_high().set_bit();
            w.thres0_low().set_bit()
        });

        ADC_MONITOR_TRIGGERED.store(false, Ordering::Release);
        ADC_MONITOR_EVENT.store(0, Ordering::Release);

        bind_and_enable_interrupt(if S::ENABLED {
            adc_monitor_interrupt_handler_tracking.handler()
        } else {
            adc_monitor_interrupt_handler_no_sample.handler()
        });

        Self {
            _adc1: adc1,
            _pin0: pin,
            _pin1: (),
            monitor_channel: channel,
            monitor1_channel: 0, // unused in Single mode
            _mode: PhantomData,
            _sample: PhantomData,
        }
    }

    pub fn set_threshold(&mut self, threshold: MonitorThreshold) {
        // SAFETY: monitor_channel was validated to 0–7 at construction time;
        // threshold values are clamped to ADC_DIGITAL_RAW_MASK (12 bits) here.
        APB_SARADC::regs().thres0_ctrl().write(|w| unsafe {
            w.thres0_channel().bits(self.monitor_channel);
            w.thres0_high()
                .bits(threshold.high.min(ADC_DIGITAL_RAW_MASK as u16));
            w.thres0_low()
                .bits(threshold.low.min(ADC_DIGITAL_RAW_MASK as u16))
        });
    }

    pub fn wait_threshold(&self) -> MonitorFuture {
        MonitorFuture { monitor: 0 }
    }
}

impl<'d, PIN0> AdcMonitor<'d, PIN0, (), Single, TrackSample>
where
    PIN0: Adc1MonitorChannel,
{
    /// Construct a single-channel monitor with last-sample tracking enabled (default).
    ///
    /// `pin` must be an [`AdcPin`] obtained from [`AdcConfig::enable_pin`] so
    /// that the GPIO is already configured for analog use.
    pub fn new<CS>(adc1: ADC1<'d>, pin: AdcPin<PIN0, ADC1<'d>, CS>, config: MonitorConfig) -> Self {
        Self::new_impl(adc1, pin, config)
    }

    pub fn last_sample(&self) -> u32 {
        ADC_MONITOR_LAST_SAMPLE.load(Ordering::Acquire)
    }
}

impl<'d, PIN0> AdcMonitor<'d, PIN0, (), Single, NoSample>
where
    PIN0: Adc1MonitorChannel,
{
    /// Construct a single-channel monitor without last-sample tracking.
    ///
    /// `pin` must be an [`AdcPin`] obtained from [`AdcConfig::enable_pin`] so
    /// that the GPIO is already configured for analog use.
    pub fn new_no_sample<CS>(
        adc1: ADC1<'d>,
        pin: AdcPin<PIN0, ADC1<'d>, CS>,
        config: MonitorConfig,
    ) -> Self {
        Self::new_impl(adc1, pin, config)
    }
}

// ---- Dual-monitor mode ----
impl<'d, PIN0, PIN1, S: SampleTracking> AdcMonitor<'d, PIN0, PIN1, Dual, S>
where
    PIN0: Adc1MonitorChannel,
    PIN1: Adc1MonitorChannel,
{
    fn new_dual_impl<CS0, CS1>(
        adc1: ADC1<'d>,
        pin0: AdcPin<PIN0, ADC1<'d>, CS0>,
        pin1: AdcPin<PIN1, ADC1<'d>, CS1>,
        config0: MonitorConfig,
        config1: MonitorConfig,
    ) -> Self {
        let AdcPin { pin: pin0, .. } = pin0;
        let AdcPin { pin: pin1, .. } = pin1;
        let channel0 = pin0.adc_channel();
        let channel1 = pin1.adc_channel();

        let atten0 = config0.attenuation as u8;
        let threshold_low0 = config0.threshold.low.min(ADC_DIGITAL_RAW_MASK as u16);
        let threshold_high0 = config0.threshold.high.min(ADC_DIGITAL_RAW_MASK as u16);

        let atten1 = config1.attenuation as u8;
        let threshold_low1 = config1.threshold.low.min(ADC_DIGITAL_RAW_MASK as u16);
        let threshold_high1 = config1.threshold.high.min(ADC_DIGITAL_RAW_MASK as u16);

        let saradc = APB_SARADC::regs();

        init_apb_saradc_clock_and_power();
        set_channel_attenuation(channel0, atten0);
        set_channel_attenuation(channel1, atten1);

        // Pattern table: 2 entries (channel0 @ bits[23:18], channel1 @ bits[17:12]).
        let patt0 = ((((channel0 & 0x0f) as u32) << 2) | ((atten0 & 0x03) as u32)) << 18;
        let patt1 = ((((channel1 & 0x0f) as u32) << 2) | ((atten1 & 0x03) as u32)) << 12;
        // SAFETY: patt0 and patt1 each pack a 4-bit channel (0–7) and a 2-bit
        // attenuation into adjacent 6-bit slots at bits [23:18] and [17:12] per
        // the ESP32-S3 TRM pattern-table format; all remaining bits are zero.
        saradc
            .sar1_patt_tab1()
            .write(|w| unsafe { w.sar1_patt_tab1().bits(patt0 | patt1) });

        configure_digital_controller(config0.sample_rate, 1); // 1 => 2 entries

        // SAFETY: channel0/channel1 are in 0–7 (enforced by Adc1MonitorChannel);
        // threshold values are clamped to ADC_DIGITAL_RAW_MASK (12 bits) above.
        // Program monitor 0 threshold.
        saradc.thres0_ctrl().write(|w| unsafe {
            w.thres0_channel().bits(channel0);
            w.thres0_high().bits(threshold_high0);
            w.thres0_low().bits(threshold_low0)
        });
        // Program monitor 1 threshold.
        saradc.thres1_ctrl().write(|w| unsafe {
            w.thres1_channel().bits(channel1);
            w.thres1_high().bits(threshold_high1);
            w.thres1_low().bits(threshold_low1)
        });
        saradc.thres_ctrl().modify(|_, w| {
            w.thres_all_en().clear_bit();
            w.thres0_en().set_bit();
            w.thres1_en().set_bit()
        });

        // Clear stale interrupts and enable both monitors' IRQs.
        saradc.int_clr().write(|w| {
            w.thres0_high().clear_bit_by_one();
            w.thres0_low().clear_bit_by_one();
            w.thres1_high().clear_bit_by_one();
            w.thres1_low().clear_bit_by_one();
            w
        });
        saradc.int_ena().modify(|_, w| {
            w.thres0_high().set_bit();
            w.thres0_low().set_bit();
            w.thres1_high().set_bit();
            w.thres1_low().set_bit()
        });

        ADC_MONITOR_TRIGGERED.store(false, Ordering::Release);
        ADC_MONITOR_EVENT.store(0, Ordering::Release);
        ADC_MONITOR1_TRIGGERED.store(false, Ordering::Release);
        ADC_MONITOR1_EVENT.store(0, Ordering::Release);

        bind_and_enable_interrupt(if S::ENABLED {
            adc_monitor_interrupt_handler_tracking.handler()
        } else {
            adc_monitor_interrupt_handler_no_sample.handler()
        });

        Self {
            _adc1: adc1,
            _pin0: pin0,
            _pin1: pin1,
            monitor_channel: channel0,
            monitor1_channel: channel1,
            _mode: PhantomData,
            _sample: PhantomData,
        }
    }

    pub fn set_threshold0(&mut self, threshold: MonitorThreshold) {
        // SAFETY: monitor_channel was validated to 0–7 at construction time;
        // threshold values are clamped to ADC_DIGITAL_RAW_MASK (12 bits) here.
        APB_SARADC::regs().thres0_ctrl().write(|w| unsafe {
            w.thres0_channel().bits(self.monitor_channel);
            w.thres0_high()
                .bits(threshold.high.min(ADC_DIGITAL_RAW_MASK as u16));
            w.thres0_low()
                .bits(threshold.low.min(ADC_DIGITAL_RAW_MASK as u16))
        });
    }

    pub fn set_threshold1(&mut self, threshold: MonitorThreshold) {
        // SAFETY: monitor1_channel was validated to 0–7 at construction time;
        // threshold values are clamped to ADC_DIGITAL_RAW_MASK (12 bits) here.
        APB_SARADC::regs().thres1_ctrl().write(|w| unsafe {
            w.thres1_channel().bits(self.monitor1_channel);
            w.thres1_high()
                .bits(threshold.high.min(ADC_DIGITAL_RAW_MASK as u16));
            w.thres1_low()
                .bits(threshold.low.min(ADC_DIGITAL_RAW_MASK as u16))
        });
    }

    pub fn wait_threshold0(&self) -> MonitorFuture {
        MonitorFuture { monitor: 0 }
    }

    pub fn wait_threshold1(&self) -> MonitorFuture {
        MonitorFuture { monitor: 1 }
    }
}

impl<'d, PIN0, PIN1> AdcMonitor<'d, PIN0, PIN1, Dual, TrackSample>
where
    PIN0: Adc1MonitorChannel,
    PIN1: Adc1MonitorChannel,
{
    /// Construct a dual-channel monitor with last-sample tracking enabled (default).
    ///
    /// `config0.sample_rate` governs the digital controller timing for both channels.
    /// Both pins must be [`AdcPin`]s obtained from [`AdcConfig::enable_pin`].
    pub fn new_dual<CS0, CS1>(
        adc1: ADC1<'d>,
        pin0: AdcPin<PIN0, ADC1<'d>, CS0>,
        pin1: AdcPin<PIN1, ADC1<'d>, CS1>,
        config0: MonitorConfig,
        config1: MonitorConfig,
    ) -> Self {
        Self::new_dual_impl(adc1, pin0, pin1, config0, config1)
    }

    pub fn last_sample0(&self) -> u32 {
        ADC_MONITOR_LAST_SAMPLE.load(Ordering::Acquire)
    }

    pub fn last_sample1(&self) -> u32 {
        ADC_MONITOR1_LAST_SAMPLE.load(Ordering::Acquire)
    }
}

impl<'d, PIN0, PIN1> AdcMonitor<'d, PIN0, PIN1, Dual, NoSample>
where
    PIN0: Adc1MonitorChannel,
    PIN1: Adc1MonitorChannel,
{
    /// Construct a dual-channel monitor without last-sample tracking.
    ///
    /// `config0.sample_rate` governs the digital controller timing for both channels.
    /// Both pins must be [`AdcPin`]s obtained from [`AdcConfig::enable_pin`].
    pub fn new_dual_no_sample<CS0, CS1>(
        adc1: ADC1<'d>,
        pin0: AdcPin<PIN0, ADC1<'d>, CS0>,
        pin1: AdcPin<PIN1, ADC1<'d>, CS1>,
        config0: MonitorConfig,
        config1: MonitorConfig,
    ) -> Self {
        Self::new_dual_impl(adc1, pin0, pin1, config0, config1)
    }
}

// ---- Drop: disable all monitors regardless of mode ----
impl<'d, PIN0, PIN1, Mode, S> Drop for AdcMonitor<'d, PIN0, PIN1, Mode, S> {
    fn drop(&mut self) {
        let saradc = APB_SARADC::regs();
        saradc.int_ena().modify(|_, w| {
            w.thres0_high().clear_bit();
            w.thres0_low().clear_bit();
            w.thres1_high().clear_bit();
            w.thres1_low().clear_bit()
        });
        saradc.thres_ctrl().modify(|_, w| {
            w.thres0_en().clear_bit();
            w.thres1_en().clear_bit()
        });
        self.stop();
    }
}

// ---- Future ----

/// A future that resolves when the ADC threshold monitor fires.
/// The `monitor` field selects which monitor (0 or 1) to wait on.
pub struct MonitorFuture {
    monitor: u8,
}

pub async fn wait_for_threshold0() -> ThresholdEvent {
    MonitorFuture { monitor: 0 }.await
}

pub async fn wait_for_threshold1() -> ThresholdEvent {
    MonitorFuture { monitor: 1 }.await
}

pub fn latest_sample0() -> u32 {
    ADC_MONITOR_LAST_SAMPLE.load(Ordering::Acquire)
}

pub fn latest_sample1() -> u32 {
    ADC_MONITOR1_LAST_SAMPLE.load(Ordering::Acquire)
}

impl Future for MonitorFuture {
    type Output = ThresholdEvent;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let (triggered, event_atomic, waker_mutex) = if self.monitor == 0 {
            (
                &ADC_MONITOR_TRIGGERED,
                &ADC_MONITOR_EVENT,
                &ADC_MONITOR_WAKER,
            )
        } else {
            (
                &ADC_MONITOR1_TRIGGERED,
                &ADC_MONITOR1_EVENT,
                &ADC_MONITOR1_WAKER,
            )
        };

        if triggered.swap(false, Ordering::AcqRel) {
            let event = event_atomic.swap(0, Ordering::AcqRel);
            if let Some(event) = ThresholdEvent::from_u8(event) {
                return Poll::Ready(event);
            }
        }

        critical_section::with(|cs| {
            *waker_mutex.borrow(cs).borrow_mut() = Some(cx.waker().clone());
        });

        // Re-check after registering to avoid a race with an IRQ that fires
        // between the first check and waker registration.
        if triggered.swap(false, Ordering::AcqRel) {
            let event = event_atomic.swap(0, Ordering::AcqRel);
            if let Some(event) = ThresholdEvent::from_u8(event) {
                return Poll::Ready(event);
            }
        }

        Poll::Pending
    }
}
