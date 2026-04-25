use core::ops::Rem;

use zerocopy::{Immutable, KnownLayout, Unaligned};

use crate::Angle;

/// Pixel reprenentation (TODO - move to using something like 256 color format?)
#[repr(C)]
#[derive(Default, Debug, KnownLayout, Immutable, Unaligned, Clone, Copy, PartialEq, Eq)]
pub struct Pixel {
    /// The red pixel SRGB value
    pub red: u8,
    /// The green pixel SRGB value
    pub green: u8,
    /// The blue pixel SRGB value
    pub blue: u8,
}

/// Defines a bitmap in pixel-polar space that allows for more easy processing of data points
#[repr(C)]
#[derive(Debug, KnownLayout, Immutable, Unaligned, Clone, Copy, PartialEq, Eq)]
pub struct PolarBitmap<const LEDS: usize, const RADIALS: usize> {
    /// Provides the pixels as a contiguous data set, with each strip of LEDs being layed out contiguously
    /// separated by the different angle measurements. Indices 0-X provide the first strip of X pixels for
    /// the first angle, 0. This continues throughout the entire circle
    pub pixels: [[Pixel; LEDS]; RADIALS],
}

impl<const LEDS: usize, const RADIALS: usize> PolarBitmap<LEDS, RADIALS> {
    /// Conversion to multiply the index by to get radians
    const IND_TO_RAD: f32 = Angle::CIRCLE.radians() / RADIALS as f32;
    /// Conversion to multiply the angle by to get the index
    const RAD_TO_IND: f32 = 1.0 / Self::IND_TO_RAD;

    /// Converts an index to radians
    pub const fn index_to_radians(i: usize) -> f32 {
        i as f32 * Self::IND_TO_RAD
    }

    /// Provides a Particular pixel value for a given angle and LED count
    pub fn get_pixel(&self, angle: Angle, num: usize) -> &Pixel {
        &self.get_pixel_strip(angle)[num]
    }

    /// Provides the entire pixel strip for a given angle
    pub fn get_pixel_strip(&self, angle: Angle) -> &[Pixel] {
        &self.pixels[((angle.radians() * Self::RAD_TO_IND) as usize).rem(RADIALS)]
    }
}

impl<const LEDS: usize, const RADIALS: usize> Default for PolarBitmap<LEDS, RADIALS> {
    fn default() -> Self {
        Self {
            pixels: [[Pixel::default(); LEDS]; RADIALS],
        }
    }
}
