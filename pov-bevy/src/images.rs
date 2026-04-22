use bevy::prelude::*;
use pov_algs::{LedGeometry, images::PolarBitmap};
use pov_images::default::{DEFAULT_IMAGE, ImageOption, create_default_images};

#[derive(Event)]
pub struct ImageChanged {
    pub name: String,
}

#[derive(Resource)]
pub struct ImageState {
    polar_selections: Vec<PolarBitmap>,
    selections: Vec<ImageOption>,
    index: usize,
}

impl ImageState {
    pub fn new<T: LedGeometry>(geometry: &T) -> Self {
        Self {
            polar_selections: vec![PolarBitmap::from_bitmap(
                &DEFAULT_IMAGE,
                geometry.led_unit_positions(),
            )],
            selections: create_default_images(),
            index: 0,
        }
    }

    pub fn current_polar(&self) -> &PolarBitmap {
        &self.polar_selections[0]
    }

    //pub fn current_image(&self) -> &Bitmap<256> {
    //    self.selections[self.index].image.current_image()
    //}

    pub fn current_name(&self) -> &str {
        &self.selections[self.index].name
    }

    pub fn next_img(&mut self) {
        let len = self.selections.len();
        self.index = (self.index + 1) % len;
    }

    pub fn step_dt(&mut self, dt: f32) {
        let idx = self.index;
        self.selections[idx].image.step_dt(dt);
    }

    pub fn step_rotation(&mut self) {
        let idx = self.index;
        self.selections[idx].image.step_rotation();
    }
}
