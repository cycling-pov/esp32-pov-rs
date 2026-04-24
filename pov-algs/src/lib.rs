#![no_std]

use core::{
    f32,
    ops::{Add, Div, Mul, Rem, Sub},
    time::Duration,
};

pub mod filters;
pub mod images;

/// Defines an angular position in radians
#[derive(Default, Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Angle(f32);

impl Angle {
    /// Provides the number of radians in one circle
    pub const CIRCLE: Angle = Angle::from_radians(2.0 * f32::consts::PI);

    /// Conversion multiplier to convert from degrees to radians
    pub const DEGREES_TO_RADIANS: f32 = f32::consts::PI / 180.0;

    /// Conversion multiplier from radians to degrees
    pub const RADIANS_TO_DEGREES: f32 = 1.0 / Self::DEGREES_TO_RADIANS;

    /// Constructs an angle in radians
    pub const fn from_radians(x: f32) -> Self {
        Self(x)
    }

    /// Constructs an angle in degrees
    pub const fn from_degrees(x: f32) -> Self {
        Self(x * Self::DEGREES_TO_RADIANS)
    }

    /// Provides the angle in radians
    pub const fn radians(self) -> f32 {
        self.0
    }

    /// Provides the angle in degrees
    pub const fn degrees(self) -> f32 {
        self.0 * Self::RADIANS_TO_DEGREES
    }

    /// Provides the angular error between [-pi, pi)
    pub fn error(a: Angle, b: Angle) -> Angle {
        Self(
            ((a.0 - b.0) + core::f32::consts::PI).rem(Self::CIRCLE.radians())
                - core::f32::consts::PI,
        )
    }

    /// Constraints to within a circle
    pub fn constrain_circle(self) -> Self {
        Self(self.0.rem(Self::CIRCLE.radians()))
    }

    /// Provides the absolute value of the current angle
    pub fn abs(self) -> Self {
        Self(self.0.abs())
    }
}

impl Add for Angle {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Angle {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl Div<Duration> for Angle {
    type Output = AngularVelocity;

    fn div(self, rhs: Duration) -> Self::Output {
        AngularVelocity(self.0 / rhs.as_secs_f32())
    }
}

/// Defines an angular rate in radians/seconds
#[derive(Default, Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct AngularVelocity(f32);

impl AngularVelocity {
    /// Creates a new angular velocity in radians/seconds
    pub const fn from_radians_secs(x: f32) -> Self {
        Self(x)
    }

    /// Creates a new angular velocity in degrees/seconds
    pub const fn from_degrees_secs(x: f32) -> Self {
        Self(x * Angle::DEGREES_TO_RADIANS)
    }

    /// Gets the current angular velocity in radians/seconds
    pub const fn radians_secs(self) -> f32 {
        self.0
    }

    /// Gets the current angular velocity in degrees/seconds
    pub const fn degrees_secs(self) -> f32 {
        self.0 * Angle::RADIANS_TO_DEGREES
    }

    /// Provides the absolute value of the current angle
    pub fn abs(self) -> Self {
        Self(self.0.abs())
    }
}

impl Add for AngularVelocity {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for AngularVelocity {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl Mul<Duration> for AngularVelocity {
    type Output = Angle;

    fn mul(self, rhs: Duration) -> Self::Output {
        Angle(self.0 * rhs.as_secs_f32())
    }
}

/// Parameters to define the LED geometry of a given wheel
pub trait LedGeometry {
    /// Defines the number of spokes
    fn num_spokes(&self) -> usize;

    /// Provides the LED dimensions in unit vectors (1 = outer dimeter, 0 = axle)
    fn led_unit_positions(&self) -> &[f32];
}

/// Defines the LED Color that is provided from a pattern controller
#[derive(Debug, Default, Copy, Clone, Eq, PartialEq)]
pub struct LedColor {
    red: u8,
    green: u8,
    blue: u8,
    alpha: u8,
}

/// Trait to allow controlling
pub trait LedImageController {
    /// Provides an LED strip vector for a given angular velocity and position
    fn get_led_vector(
        &self,
        angular_velocity: AngularVelocity,
        angular_position: Angle,
        led_colors: &mut [LedColor],
    );
}
