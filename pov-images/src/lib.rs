use std::{io::Cursor, path::Path};

use image::GenericImageView;

pub fn image_from_file<const N: usize>(file: &Path) -> pov_algs::images::Bitmap<N> {
    let mut img = pov_algs::images::Bitmap::<N>::default();
    let img_load = image::ImageReader::open(file).unwrap();
    let dyni = img_load.decode().unwrap();

    for i in 0..N {
        for j in 0..N {
            let pxval = dyni.get_pixel(i as u32, j as u32).0;
            img.pixels[i][j] = pov_algs::images::Pixel {
                red: pxval[0],
                green: pxval[1],
                blue: pxval[2],
            };
        }
    }

    img
}

pub fn image_from_data<const N: usize>(data: &[u8]) -> pov_algs::images::Bitmap<N> {
    let mut img = pov_algs::images::Bitmap::<N>::default();
    let img_load = image::ImageReader::new(Cursor::new(data))
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
