use bevy::prelude::*;
use pov_algs::LedGeometry;
use pov_images::{
    DefaultImageType,
    default::{ImageOption, create_default_images},
};

#[derive(Event)]
pub struct ImageChanged {
    pub name: String,
}

#[derive(Resource)]
pub struct ImageState {
    selections: Vec<ImageOption>,
    index: usize,
}

impl ImageState {
    pub fn new<T: LedGeometry>(geometry: &T) -> Self {
        Self {
            selections: create_default_images(geometry.led_unit_positions()),
            index: 0,
        }
    }

    pub fn current_image(&self) -> &DefaultImageType {
        self.selections[self.index].image.current_image()
    }

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
