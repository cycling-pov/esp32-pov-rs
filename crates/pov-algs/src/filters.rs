use core::time::Duration;

use crate::{Angle, AngularVelocity};

/// Implements a naive first-order low-pass filter
#[derive(Debug, Default, Clone, Copy)]
pub struct LowPassFilter {
    /// The value used as state
    value: f32,
    /// The time constant
    tau: Duration,
}

impl LowPassFilter {
    /// Creates the low pass filter with a given time constant
    pub const fn new(tau: Duration) -> Self {
        Self { value: 0.0, tau }
    }

    /// Steps the low-pass filter for an input value with a given delta time
    pub const fn step(&mut self, val: f32, dt: Duration) -> f32 {
        self.value = self.value + dt.as_secs_f32() / self.tau.as_secs_f32() * (val - self.value);
        self.value
    }

    /// Resets the low-pass filter to 0
    pub const fn reset(&mut self) {
        self.value = 0.0;
    }

    /// Resets the low-pass filter to a given value
    pub const fn reset_value(&mut self, val: f32) {
        self.value = val;
    }

    /// Obtains the current value without stepping the filter
    pub const fn get_value(&self) -> f32 {
        self.value
    }
}

/// Implements a simple moving-average filter
#[derive(Debug, Clone, Copy)]
pub struct MovingAverageFilter<const N: usize> {
    /// The stored array of values
    values: [f32; N],
    /// The current sum
    sum: f32,
    /// The current index
    current_index: usize,
}

impl<const N: usize> MovingAverageFilter<N> {
    /// Updates the filter and returns the current value
    pub fn step(&mut self, value: f32) -> f32 {
        self.sum += (value - self.values[self.current_index]) / (N as f32);
        self.values[self.current_index] = value;
        self.current_index = (self.current_index + 1) % N;
        self.sum
    }

    /// Gets the current sum without updating
    pub fn get_value(&self) -> f32 {
        self.sum
    }

    /// Resets the filter
    pub fn reset(&mut self) {
        self.values = [0.0; N];
        self.sum = 0.0;
        self.current_index = 0;
    }
}

impl<const N: usize> Default for MovingAverageFilter<N> {
    fn default() -> Self {
        assert_ne!(N, 0);
        Self {
            values: [0.0; N],
            sum: 0.0,
            current_index: 0,
        }
    }
}

/// Implements a simple position estimator
#[derive(Debug, Clone, Copy)]
pub struct PositionEstimator<const SPOKES: usize> {
    /// The low-pass filter to use from the "tick" of the position tick
    period_filter: LowPassFilter,
    /// Provides a history estimate for the moving average filter
    period_history: MovingAverageFilter<SPOKES>,
    /// Defines the sum of the current period
    current_period: Duration,
    /// Defines the current rate
    rate: AngularVelocity,
    /// Defines the current position
    pos: Angle,
    /// Defines spoke positions
    spoke_pos: [Angle; SPOKES],
    /// Define the last spoke position hit
    last_spoke: usize,
}

impl<const SPOKES: usize> PositionEstimator<SPOKES> {
    const SPOKE_OFFSET: Angle = Angle::from_radians(Angle::CIRCLE.radians() / (SPOKES as f32));

    /// Resets the estimator to 0.0
    pub fn reset(&mut self) {
        self.current_period = Duration::ZERO;
        self.rate = AngularVelocity::default();
        self.pos = Angle::default();
        self.period_filter.reset();
    }

    /// Provides the current position
    pub fn get_current_pos(&self) -> Angle {
        self.pos
    }

    /// Provides the current rate
    pub fn get_current_rate(&self) -> AngularVelocity {
        self.rate
    }

    /// Steps the estimator
    pub fn step(&mut self, dt: Duration, triggered: Option<usize>) {
        // Update the spoke index for the last read position
        if let Some(ind) = triggered {
            self.period_history.step(self.current_period.as_secs_f32());
            self.last_spoke = ind;
            self.current_period = Duration::ZERO;
        }

        // Update the period
        self.current_period += dt;

        // Update the rate based on the period filter
        self.rate = AngularVelocity::from_radians_secs(
            if self.period_filter.step(self.period_history.get_value(), dt) > 0.1 {
                Angle::CIRCLE.radians() / self.period_filter.get_value() / SPOKES as f32
            } else {
                0.0
            },
        );

        // Update the position as a rate with the period and base position from the last spoke provided
        self.pos =
            (self.spoke_pos[self.last_spoke] + self.rate * self.current_period).constrain_circle();
    }
}

impl<const SPOKES: usize> Default for PositionEstimator<SPOKES> {
    fn default() -> Self {
        // Assume that marker spokes are equidistant
        let mut pos_vals = [Angle::default(); SPOKES];
        for (i, x) in pos_vals.iter_mut().enumerate() {
            *x = Angle::from_radians(Self::SPOKE_OFFSET.radians() * (i as f32));
        }

        // Construct the filter
        Self {
            period_filter: LowPassFilter::new(Duration::from_secs_f32(2.0)),
            period_history: MovingAverageFilter::<SPOKES>::default(),
            current_period: Duration::ZERO,
            rate: AngularVelocity::default(),
            pos: Angle::default(),
            spoke_pos: pos_vals,
            last_spoke: 0,
        }
    }
}
