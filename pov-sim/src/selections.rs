use pov_algs::images::Bitmap;

/// Generic image selection for processing in the event loop
pub trait ImageSelection {
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
