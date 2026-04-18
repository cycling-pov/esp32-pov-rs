use std::sync::{Arc, LazyLock, Mutex};

use bevy::prelude::*;
use pov_algs::images::Bitmap;
use pov_images::{
    Image, ImageSelection, VideoRotation, VideoTime, frames_from_data, image_from_data,
};

#[derive(Resource)]
pub struct ImageState {
    pub selections: Vec<(String, Box<dyn ImageSelection>)>,
    pub index: usize,
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
                ("earth".into(), Box::new(Image::new(&EARTH_IMG))),
                (
                    "cat (rot)".into(),
                    Box::new(VideoRotation::new(CAT_FRAMES.as_slice())),
                ),
                (
                    "cat (dt)".into(),
                    Box::new(VideoTime::new(CAT_FRAMES.as_slice(), 0.05)),
                ),
            ],
            index: 0,
        }
    }
}
