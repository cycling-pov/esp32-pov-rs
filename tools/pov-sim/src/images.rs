use std::{sync::Arc, time::Duration};

use bevy::prelude::*;
use pov_algs::LedGeometry;
use pov_images::{
    DefaultImageType, ImageSelection, StaticImage, VideoRotation, VideoTime, frames_from_file,
    image_from_file,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Event)]
pub struct ImageChanged {
    pub name: String,
}

pub struct ImageOption {
    pub name: String,
    pub image: Box<dyn ImageSelection>,
}

#[derive(Resource)]
pub struct ImageState {
    selections: Vec<ImageOption>,
    index: usize,
}

impl ImageState {
    pub fn new<T: LedGeometry>(geometry: &T, config: &ImageConfig) -> Self {
        let mut selections = Vec::new();

        for c in &config.images {
            let opt = match c.img_type {
                ImageConfigType::Static => ImageOption {
                    name: c.name.clone(),
                    image: Box::new(StaticImage::new(image_from_file(
                        &c.path,
                        geometry.led_unit_positions(),
                    ))),
                },
                ImageConfigType::MovieRotation => ImageOption {
                    name: c.name.clone(),
                    image: Box::new(VideoRotation::new(
                        frames_from_file(&c.path, geometry.led_unit_positions())
                            .into_iter()
                            .map(Arc::new)
                            .collect(),
                    )),
                },
                ImageConfigType::MovieTime(dt) => ImageOption {
                    name: c.name.clone(),
                    image: Box::new(VideoTime::new(
                        frames_from_file(&c.path, geometry.led_unit_positions())
                            .into_iter()
                            .map(Arc::new)
                            .collect(),
                        Duration::from_secs_f64(dt),
                    )),
                },
            };

            selections.push(opt);
        }

        Self {
            selections,
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

    pub fn step_dt(&mut self, dt: Duration) {
        let idx = self.index;
        self.selections[idx].image.step_dt(dt);
    }

    pub fn step_rotation(&mut self) {
        let idx = self.index;
        self.selections[idx].image.step_rotation();
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageConfig {
    pub images: Vec<ImageConfigEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageConfigEntry {
    pub name: String,
    pub path: PathBuf,
    pub img_type: ImageConfigType,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ImageConfigType {
    Static,
    MovieRotation,
    MovieTime(f64),
}
