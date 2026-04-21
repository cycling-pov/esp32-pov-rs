use std::{
    fs::File,
    io::{BufRead, BufReader, Cursor, Seek},
    path::Path,
};

use image::{AnimationDecoder, codecs::gif::GifDecoder, imageops::resize};
use pov_algs::images::{Bitmap, PolarBitmap};

fn read_single_image<T: BufRead + Seek, const N: usize>(input: T) -> Bitmap<N> {
    let mut img = Bitmap::<N>::default();
    let img_load = image::ImageReader::new(input)
        .with_guessed_format()
        .unwrap()
        .decode()
        .unwrap();
    let img_sized = resize(
        &img_load,
        N as u32,
        N as u32,
        image::imageops::FilterType::Triangle,
    );

    for i in 0..N {
        for j in 0..N {
            let pxval = img_sized.get_pixel(i as u32, j as u32).0;
            img.pixels[i][j] = pov_algs::images::Pixel {
                red: pxval[0],
                green: pxval[1],
                blue: pxval[2],
            };
        }
    }

    img
}

pub fn image_from_file<const N: usize>(file: &Path) -> Bitmap<N> {
    read_single_image(BufReader::new(File::open(file).unwrap()))
}

pub fn image_from_data<const N: usize>(data: &[u8]) -> Bitmap<N> {
    read_single_image(Cursor::new(data))
}

fn read_gif<T: BufRead + Seek, const N: usize>(input: T) -> Vec<Bitmap<N>> {
    let img_load = GifDecoder::new(input)
        .unwrap()
        .into_frames()
        .collect_frames()
        .unwrap();

    let mut output = Vec::new();

    for f in img_load {
        let buf = resize(
            f.buffer(),
            N as u32,
            N as u32,
            image::imageops::FilterType::Triangle,
        );

        let mut img = pov_algs::images::Bitmap::<N>::default();

        for i in 0..N {
            for j in 0..N {
                let pxval = buf.get_pixel(i as u32, j as u32).0;
                img.pixels[i][j] = pov_algs::images::Pixel {
                    red: pxval[0],
                    green: pxval[1],
                    blue: pxval[2],
                };
            }
        }

        output.push(img);
    }

    output
}

pub fn frames_from_file<const N: usize>(file: &Path) -> Vec<pov_algs::images::Bitmap<N>> {
    read_gif(BufReader::new(File::open(file).unwrap()))
}

pub fn frames_from_data<const N: usize>(data: &[u8]) -> Vec<pov_algs::images::Bitmap<N>> {
    read_gif(BufReader::new(Cursor::new(data)))
}

/// Defines the default image type
type ImageType = Bitmap<256>;

/// Generic image selection for processing in the event loop
pub trait ImageSelection: Send + Sync {
    fn current_image(&self) -> &ImageType;
    fn step_dt(&mut self, dt: f32);
    fn step_rotation(&mut self);
}

/// Implements a static image
pub struct Image<'a> {
    image: &'a ImageType,
}

impl<'a> Image<'a> {
    pub fn new(image: &'a ImageType) -> Self {
        Self { image }
    }
}

impl<'a> ImageSelection for Image<'a> {
    fn current_image(&self) -> &ImageType {
        self.image
    }

    fn step_dt(&mut self, _dt: f32) {}

    fn step_rotation(&mut self) {}
}

/// Implements a video that increments once per wheel rotation
pub struct VideoRotation<'a> {
    images: &'a [ImageType],
    index: usize,
}

impl<'a> VideoRotation<'a> {
    pub fn new(images: &'a [ImageType]) -> Self {
        Self { images, index: 0 }
    }
}

impl<'a> ImageSelection for VideoRotation<'a> {
    fn current_image(&self) -> &ImageType {
        &self.images[self.index]
    }

    fn step_dt(&mut self, _dt: f32) {}

    fn step_rotation(&mut self) {
        self.index = (self.index + 1) % self.images.len();
    }
}

/// Implements a video that increments frames based on timing
pub struct VideoTime<'a> {
    images: &'a [ImageType],
    index: usize,
    frame_time: f32,
    current_time: f32,
}

impl<'a> VideoTime<'a> {
    pub fn new(images: &'a [ImageType], frame_time: f32) -> Self {
        Self {
            images,
            index: 0,
            frame_time,
            current_time: 0.0,
        }
    }
}

impl<'a> ImageSelection for VideoTime<'a> {
    fn current_image(&self) -> &ImageType {
        &self.images[self.index]
    }

    fn step_dt(&mut self, dt: f32) {
        self.current_time += dt;
        while self.current_time >= self.frame_time {
            self.current_time -= self.frame_time;
            self.index = (self.index + 1) % self.images.len();
        }
    }

    fn step_rotation(&mut self) {}
}

#[cfg(feature = "default-images")]
pub mod default {
    use crate::{Image, ImageSelection, VideoRotation, VideoTime};
    use std::sync::LazyLock;

    use super::{Bitmap, frames_from_data, image_from_data};

    pub static DEFAULT_IMAGE: LazyLock<super::Bitmap<256>> =
        LazyLock::new(|| image_from_data::<256>(include_bytes!("../earth.jpg")));
    pub static DEFAULT_MOVIE: LazyLock<Vec<Bitmap<256>>> =
        LazyLock::new(|| frames_from_data::<256>(include_bytes!("../cat-space.gif")));

    pub struct ImageOption {
        pub name: String,
        pub image: Box<dyn ImageSelection>,
    }

    pub fn create_default_images() -> Vec<ImageOption> {
        vec![
            ImageOption {
                name: "earth".into(),
                image: Box::new(Image::new(&DEFAULT_IMAGE)),
            },
            ImageOption {
                name: "cat (rot)".into(),
                image: Box::new(VideoRotation::new(DEFAULT_MOVIE.as_slice())),
            },
            ImageOption {
                name: "cat (dt)".into(),
                image: Box::new(VideoTime::new(DEFAULT_MOVIE.as_slice(), 0.05)),
            },
        ]
    }
}
