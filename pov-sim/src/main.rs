mod state;

use crate::state::RotationState;
use pov_images::{
    Image, ImageSelection, VideoRotation, VideoTime, frames_from_data, image_from_data,
};
use raylib::prelude::*;

struct LedValue {
    loc: (f32, f32),
    fade_val: f32,
    offset: f32,
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
    let cat_frames = frames_from_data::<256>(include_bytes!("../cat-space.gif"));

    let mut selections: Vec<(&'static str, Box<dyn ImageSelection>)> = vec![
        ("earth", Box::new(Image::new(&img_val))),
        ("cat (rot)", Box::new(VideoRotation::new(&cat_frames))),
        ("cat (dt)", Box::new(VideoTime::new(&cat_frames, 0.05))),
    ];
    let mut selection_index = 0;

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
                offset: angle,
                //id: i,
                //radius,
            });
        }
    }

    const ROT_RATE_MIN: f32 = 1.0;
    const ROT_RATE_MAX: f32 = 20.0;
    const ROT_ACCEL: f32 = 2.0;

    const FADE_TIME_MIN: f32 = 0.1;
    const FADE_TIME_MAX: f32 = 2.0;
    const FADE_TIME_RATE: f32 = 0.5;

    let mut state = RotationState::new(2, 12.0);
    let mut fade_time = 0.2f32;

    while !rl.window_should_close() {
        let scale = rl.get_window_scale_dpi();
        let cx = (rl.get_render_width() as f32 / 2.0 / scale.x) as i32;
        let cy = (rl.get_render_height() as f32 / 2.0 / scale.y) as i32;

        let wheel_radius = cx.min(cy) as f32 * 0.9;
        let wheel_inner_radius = wheel_radius * 0.95;

        state.step(rl.get_frame_time());
        if state.has_rotated() {
            selections[selection_index].1.step_rotation();
        }
        selections[selection_index].1.step_dt(rl.get_frame_time());

        let (name, val) = &selections[selection_index];
        let current = val.current_image();

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

            let px = current.get_nearest(l.loc.0, l.loc.1);
            let color = Color::new(px.red, px.green, px.blue, 255).alpha(l.fade_val);

            d.draw_circle(
                cx + (l.loc.0 * wheel_inner_radius) as i32,
                cy + (l.loc.1 * wheel_inner_radius) as i32,
                2.0,
                color,
            );
        }

        let mut rot_dir: i32 = 0;

        if d.is_key_down(KeyboardKey::KEY_UP) {
            rot_dir += 1;
        }

        if d.is_key_down(KeyboardKey::KEY_DOWN) {
            rot_dir -= 1;
        }

        state.rotation_rate = (state.rotation_rate
            + rot_dir as f32 * ROT_ACCEL * d.get_frame_time())
        .clamp(ROT_RATE_MIN, ROT_RATE_MAX);

        let mut fade_dir: i32 = 0;

        if d.is_key_down(KeyboardKey::KEY_RIGHT) {
            fade_dir += 1;
        }

        if d.is_key_down(KeyboardKey::KEY_LEFT) {
            fade_dir -= 1;
        }
        fade_time = (fade_time + fade_dir as f32 * FADE_TIME_RATE * d.get_frame_time())
            .clamp(FADE_TIME_MIN, FADE_TIME_MAX);

        if d.is_key_pressed(KeyboardKey::KEY_A) {
            selection_index = (selection_index + 1) % selections.len();
        }

        d.draw_text(
            &format!(
                "Speed: {:.2} rad/s (Up/Down)\nFade: {:.2} s (Left/Right)\nFPS: {}\n{}\n'a' to advance",
                state.rotation_rate,
                fade_time,
                d.get_fps(),
                name
            ),
            12,
            12,
            20,
            Color::BLACK,
        );
    }
}
