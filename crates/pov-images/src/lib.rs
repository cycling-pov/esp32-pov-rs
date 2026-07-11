use std::{
    fs::File,
    io::{BufRead, BufReader, Cursor, Seek},
    path::Path,
    sync::Arc,
    time::Duration,
};

use image::{AnimationDecoder, Pixel, RgbaImage, codecs::gif::GifDecoder};
use pov_algs::images::PolarBitmap;

/// Constructs the polar bitmap from a square bitmap with provided LED radii, in percentages from [0, 1],
/// from the center of the bitmap
pub fn polar_from_image<const N: usize, const R: usize>(
    image: &RgbaImage,
    radii: &[f32],
) -> PolarBitmap<N, R> {
    let mut polar = PolarBitmap::<N, R>::default();

    fn get_nearest(image: &RgbaImage, x: f32, y: f32) -> pov_algs::images::Pixel {
        let width = image.width();
        let height = image.height();

        let diff_x: f32 = (width / 2) as f32;
        let diff_y: f32 = (height / 2) as f32;
        let xi = ((diff_x * x + diff_x) as i32).clamp(0, width as i32 - 1) as u32;
        let yi = ((diff_y * y + diff_y) as i32).clamp(0, height as i32 - 1) as u32;
        let px = image.get_pixel(xi, yi).to_rgb();

        pov_algs::images::Pixel {
            red: px[0],
            green: px[1],
            blue: px[2],
        }
    }

    for i in 0..R {
        let (s, c) = PolarBitmap::<N, R>::index_to_radians(i).sin_cos();

        for j in 0..N {
            let r = radii[j];
            polar.pixels[i][j] = get_nearest(image, r * c, r * s);
        }
    }

    polar
}

fn read_single_image<T: BufRead + Seek, const N: usize, const R: usize>(
    input: T,
    radii: &[f32],
) -> PolarBitmap<N, R> {
    let img_load = image::ImageReader::new(input)
        .with_guessed_format()
        .unwrap()
        .decode()
        .unwrap();

    polar_from_image(&img_load.into(), radii)
}

pub fn image_from_file<const N: usize, const R: usize>(
    file: &Path,
    radii: &[f32],
) -> PolarBitmap<N, R> {
    read_single_image(BufReader::new(File::open(file).unwrap()), radii)
}

pub fn image_from_data<const N: usize, const R: usize>(
    data: &[u8],
    radii: &[f32],
) -> PolarBitmap<N, R> {
    read_single_image(Cursor::new(data), radii)
}

fn read_gif<T: BufRead + Seek, const N: usize, const R: usize>(
    input: T,
    radii: &[f32],
) -> Vec<PolarBitmap<N, R>> {
    let img_load = GifDecoder::new(input)
        .unwrap()
        .into_frames()
        .collect_frames()
        .unwrap();

    img_load
        .into_iter()
        .map(|x| polar_from_image(x.buffer(), radii))
        .collect()
}

pub fn frames_from_file<const N: usize, const R: usize>(
    file: &Path,
    radii: &[f32],
) -> Vec<PolarBitmap<N, R>> {
    read_gif(BufReader::new(File::open(file).unwrap()), radii)
}

pub fn frames_from_data<const N: usize, const R: usize>(
    data: &[u8],
    radii: &[f32],
) -> Vec<PolarBitmap<N, R>> {
    read_gif(BufReader::new(Cursor::new(data)), radii)
}

/// The default number of LEDS
pub const DEFAULT_LEDS: usize = 30;

/// Defines the default image type
pub type DefaultImageType = PolarBitmap<DEFAULT_LEDS, 360>;

/// Generic image selection for processing in the event loop
pub trait ImageSelection: Send + Sync {
    fn current_image(&self) -> &DefaultImageType;
    fn step_dt(&mut self, dt: Duration);
    fn step_rotation(&mut self);
}

/// Implements a static image
pub struct StaticImage {
    image: DefaultImageType,
}

impl StaticImage {
    pub fn new(image: DefaultImageType) -> Self {
        Self { image }
    }
}

impl<'a> ImageSelection for StaticImage {
    fn current_image(&self) -> &DefaultImageType {
        &self.image
    }

    fn step_dt(&mut self, _dt: Duration) {}

    fn step_rotation(&mut self) {}
}

/// Implements a video that increments once per wheel rotation
pub struct VideoRotation {
    images: Vec<Arc<DefaultImageType>>,
    index: usize,
}

impl VideoRotation {
    pub fn new(images: Vec<Arc<DefaultImageType>>) -> Self {
        Self { images, index: 0 }
    }
}

impl ImageSelection for VideoRotation {
    fn current_image(&self) -> &DefaultImageType {
        &self.images[self.index]
    }

    fn step_dt(&mut self, _dt: Duration) {}

    fn step_rotation(&mut self) {
        self.index = (self.index + 1) % self.images.len();
    }
}

/// Implements a video that increments frames based on timing
pub struct VideoTime {
    images: Vec<Arc<DefaultImageType>>,
    index: usize,
    frame_time: Duration,
    current_time: Duration,
}

impl<'a> VideoTime {
    pub fn new(images: Vec<Arc<DefaultImageType>>, frame_time: Duration) -> Self {
        Self {
            images,
            index: 0,
            frame_time,
            current_time: Duration::ZERO,
        }
    }
}

impl ImageSelection for VideoTime {
    fn current_image(&self) -> &DefaultImageType {
        &self.images[self.index]
    }

    fn step_dt(&mut self, dt: Duration) {
        self.current_time += dt;
        while self.current_time >= self.frame_time {
            self.current_time -= self.frame_time;
            self.index = (self.index + 1) % self.images.len();
        }
    }

    fn step_rotation(&mut self) {}
}
