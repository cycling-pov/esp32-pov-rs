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
