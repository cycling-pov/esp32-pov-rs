use pov_images::image_from_data;
use raylib::prelude::*;
use std::ops::Rem;

struct LedValue {
    loc: (f32, f32),
    fade_val: f32,
    offset: f32,
    id: u32,
    radius: f32,
}

#[derive(Debug, Default, Clone, Copy)]
struct SpokePosition {
    current_pos: f32,
    previous_pos: f32,
}

#[derive(Debug, Clone)]
struct RotationState {
    rotation_rate: f32,
    spoke_positions: Vec<SpokePosition>,
}

impl RotationState {
    const FULL_CIRCLE: f32 = 2.0 * ::core::f32::consts::PI;

    fn new(num_spokes: usize, init_rate: f32) -> Self {
        assert!(num_spokes > 0);
        let mut s = Self {
            rotation_rate: init_rate,
            spoke_positions: vec![SpokePosition::default(); num_spokes],
        };
        s.reset();
        s
    }

    fn reset(&mut self) {
        let offset = self.offset_angle();
        for (i, pos) in self.spoke_positions.iter_mut().enumerate() {
            let current = offset * (i as f32);
            pos.current_pos = current;
            pos.previous_pos = current;
        }
    }

    fn step(&mut self, dt: f32) {
        let angle_offset = self.offset_angle();

        let init_angle = if let Some(pos) = self.spoke_positions.first_mut() {
            pos.previous_pos = pos.current_pos;
            pos.current_pos = (pos.current_pos + dt * self.rotation_rate).rem(Self::FULL_CIRCLE);
            pos.current_pos
        } else {
            panic!("unable to get spoke values");
        };

        for pos in &mut self.spoke_positions[1..] {
            pos.previous_pos = pos.current_pos;
            pos.current_pos = (init_angle + angle_offset).rem(Self::FULL_CIRCLE);
        }
    }

    fn offset_angle(&self) -> f32 {
        let num_spokes = self.spoke_positions.len();
        assert!(num_spokes > 0);
        Self::FULL_CIRCLE / num_spokes as f32
    }

    fn contains(&self, x: f32) -> bool {
        for spoke in &self.spoke_positions {
            let res = if spoke.current_pos > spoke.previous_pos {
                x >= spoke.previous_pos && x <= spoke.current_pos
            } else {
                x <= spoke.current_pos || x >= spoke.previous_pos
            };

            if res {
                return true;
            }
        }

        false
    }
}

pub fn main() {
    let (mut rl, thread) = raylib::init()
        .msaa_4x()
        .size(800, 600)
        .title("pov-sim")
        .resizable()
        .build();

    const NUM_LED: u32 = 40;
    const HUB_PERC: f32 = 0.2;
    const NUM_LED_SPOKES: u32 = 72 * 2;

    let img_val = image_from_data::<256>(include_bytes!("../earth.jpg"));

    let mut leds = Vec::new();
    for d in 0..NUM_LED_SPOKES {
        let angle = (d as f32 * 360.0 / NUM_LED_SPOKES as f32) * ::core::f32::consts::PI / 180.0;
        let (s, c) = angle.sin_cos();

        for i in 0..NUM_LED {
            let radius_perc = i as f32 / NUM_LED as f32;
            let radius_mod = radius_perc.powf(0.8);

            let radius = HUB_PERC + (1.0 - HUB_PERC) * radius_mod;
            leds.push(LedValue {
                loc: (radius * c, radius * s),
                fade_val: 1.0,
                id: i,
                offset: angle,
                radius,
            });
        }
    }

    let mut state = RotationState::new(2, 5.0);
    let mut fade_time = 0.2f32;

    while !rl.window_should_close() {
        let scale = rl.get_window_scale_dpi();
        let cx = (rl.get_render_width() as f32 / 2.0 / scale.x) as i32;
        let cy = (rl.get_render_height() as f32 / 2.0 / scale.y) as i32;

        let wheel_radius = cx.min(cy) as f32 * 0.9;
        let wheel_inner_radius = wheel_radius * 0.95;

        state.step(rl.get_frame_time());

        let mut d = rl.begin_drawing(&thread);

        const BACK_COLOR: Color = Color::DARKGRAY;

        d.clear_background(BACK_COLOR);
        d.draw_circle(cx, cy, wheel_radius, Color::BLACK);
        d.draw_circle(cx, cy, wheel_inner_radius, BACK_COLOR);
        d.draw_circle(cx, cy, HUB_PERC * wheel_inner_radius * 0.8, Color::BLACK);

        for l in &mut leds {
            if state.contains(l.offset) {
                l.fade_val = 1.0;
            } else {
                l.fade_val = (l.fade_val - d.get_frame_time() * (1.0 / fade_time)).max(0.0);
            }

            let px = img_val.get_nearest(l.loc.0, l.loc.1);
            let color = Color::new(px.red, px.green, px.blue, 255).alpha(l.fade_val);

            d.draw_circle(
                cx + (l.loc.0 * wheel_inner_radius) as i32,
                cy + (l.loc.1 * wheel_inner_radius) as i32,
                2.0,
                color,
            );
        }

        if d.is_key_down(KeyboardKey::KEY_UP) {
            state.rotation_rate = (state.rotation_rate + 2.0 * d.get_frame_time()).min(15.0);
        }

        if d.is_key_down(KeyboardKey::KEY_DOWN) {
            state.rotation_rate = (state.rotation_rate - 2.0 * d.get_frame_time()).max(1.0);
        }

        if d.is_key_down(KeyboardKey::KEY_RIGHT) {
            fade_time = (fade_time + 0.5 * d.get_frame_time()).min(2.0);
        }

        if d.is_key_down(KeyboardKey::KEY_LEFT) {
            fade_time = (fade_time - 0.5 * d.get_frame_time()).max(0.1);
        }

        d.draw_text(
            &format!(
                "Speed: {:.2} rad/s (Up/Down)\nFade: {:.2} s (Left/Right)\nFPS: {}",
                state.rotation_rate,
                fade_time,
                d.get_fps()
            ),
            12,
            12,
            20,
            Color::BLACK,
        );
    }
}
