pub struct LowPassFilter {
    value: f32,
    tau: f32,
}

impl LowPassFilter {
    pub const fn new(tau: f32) -> Self {
        Self { value: 0.0, tau }
    }

    pub const fn step(&mut self, val: f32, dt: f32) {
        self.value = self.value + dt / self.tau * (val - self.value);
    }

    pub const fn reset(&mut self) {
        self.value = 0.0;
    }

    pub const fn reset_value(&mut self, val: f32) {
        self.value = val;
    }

    pub const fn get_value(&self) -> f32 {
        self.value
    }
}

pub struct PositionEstimator {
    period_estimator: LowPassFilter,
    current_period: f32,
    rate: f32,
    pos: f32,
}

impl PositionEstimator {
    pub fn new(tau: f32) -> Self {
        Self {
            period_estimator: LowPassFilter::new(tau),
            rate: 0.0,
            pos: 0.0,
            current_period: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.current_period = 0.0;
        self.rate = 0.0;
        self.pos = 0.0;
        self.period_estimator.reset();
    }

    pub fn get_current_pos(&self) -> f32 {
        self.pos
    }

    pub fn get_current_rate(&self) -> f32 {
        self.rate
    }

    pub fn step(&mut self, dt: f32) {
        self.current_period += dt;
    }

    pub fn trigger_val(&mut self) {
        self.period_estimator
            .step(self.current_period, self.current_period);
    }
}
