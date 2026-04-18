use std::{
    fs::File,
    io::{BufRead, BufReader, Cursor, Seek},
    path::Path,
};

use image::{AnimationDecoder, GenericImageView, codecs::gif::GifDecoder};
use pov_algs::images::Bitmap;

fn read_single_image<T: BufRead + Seek, const N: usize>(input: T) -> Bitmap<N> {
    let mut img = pov_algs::images::Bitmap::<N>::default();
    let img_load = image::ImageReader::new(input)
        .with_guessed_format()
        .unwrap()
        .decode()
        .unwrap();

    for i in 0..N {
        for j in 0..N {
            let pxval = img_load.get_pixel(i as u32, j as u32).0;
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
        let buf = f.buffer();
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

/// Generic image selection for processing in the event loop
pub trait ImageSelection: Send + Sync {
    fn current_image(&self) -> &Bitmap<256>;
    fn step_dt(&mut self, dt: f32);
    fn step_rotation(&mut self);
}

/// Implements a static image
pub struct Image<'a> {
    image: &'a Bitmap<256>,
}

impl<'a> Image<'a> {
    pub fn new(image: &'a Bitmap<256>) -> Self {
        Self { image }
    }
}

impl<'a> ImageSelection for Image<'a> {
    fn current_image(&self) -> &Bitmap<256> {
        &self.image
    }

    fn step_dt(&mut self, _dt: f32) {}

    fn step_rotation(&mut self) {}
}

/// Implements a video that increments once per wheel rotation
pub struct VideoRotation<'a> {
    images: &'a [Bitmap<256>],
    index: usize,
}

impl<'a> VideoRotation<'a> {
    pub fn new(images: &'a [Bitmap<256>]) -> Self {
        Self { images, index: 0 }
    }
}

impl<'a> ImageSelection for VideoRotation<'a> {
    fn current_image(&self) -> &Bitmap<256> {
        &self.images[self.index]
    }

    fn step_dt(&mut self, _dt: f32) {}

    fn step_rotation(&mut self) {
        self.index = (self.index + 1) % self.images.len();
    }
}

/// Implements a video that increments frames based on timing
pub struct VideoTime<'a> {
    images: &'a [Bitmap<256>],
    index: usize,
    frame_time: f32,
    current_time: f32,
}

impl<'a> VideoTime<'a> {
    pub fn new(images: &'a [Bitmap<256>], frame_time: f32) -> Self {
        Self {
            images,
            index: 0,
            frame_time,
            current_time: 0.0,
        }
    }
}

impl<'a> ImageSelection for VideoTime<'a> {
    fn current_image(&self) -> &Bitmap<256> {
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
