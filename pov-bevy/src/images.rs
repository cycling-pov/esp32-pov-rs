use std::sync::LazyLock;

use bevy::prelude::*;
use pov_algs::images::Bitmap;
use pov_images::{
    Image, ImageSelection, VideoRotation, VideoTime, frames_from_data, image_from_data,
};

#[derive(Event)]
pub struct ImageChanged {
    pub name: String,
}

struct ImageOption {
    pub name: String,
    pub image: Box<dyn ImageSelection>,
}

#[derive(Resource)]
pub struct ImageState {
    selections: Vec<ImageOption>,
    index: usize,
}

impl ImageState {
    pub fn current_image(&self) -> &Bitmap<256> {
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

impl Default for ImageState {
    fn default() -> Self {
        static EARTH_IMG: LazyLock<Bitmap<256>> =
            LazyLock::new(|| image_from_data::<256>(include_bytes!("../../pov-sim/earth.jpg")));

        static CAT_FRAMES: LazyLock<Vec<Bitmap<256>>> = LazyLock::new(|| {
            frames_from_data::<256>(include_bytes!("../../pov-sim/cat-space.gif"))
        });

        Self {
            selections: vec![
                ImageOption {
                    name: "earth".into(),
                    image: Box::new(Image::new(&EARTH_IMG)),
                },
                ImageOption {
                    name: "cat (rot)".into(),
                    image: Box::new(VideoRotation::new(CAT_FRAMES.as_slice())),
                },
                ImageOption {
                    name: "cat (dt)".into(),
                    image: Box::new(VideoTime::new(CAT_FRAMES.as_slice(), 0.05)),
                },
            ],
            index: 0,
        }
    }
}
