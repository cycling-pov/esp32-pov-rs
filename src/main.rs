use raylib::prelude::*;
use std::ops::Rem;

struct LedValue {
    loc: (f32, f32),
    fade_val: f32,
    offset: f32,
    id: u32,
}

#[derive(Debug)]
struct RotationState {
    rotation_rate: f32,
    previous_pos: f32,
    current_pos: f32,
}

impl RotationState {
    fn step(&mut self, dt: f32) {
        self.previous_pos = self.current_pos;
        self.current_pos =
            (self.current_pos + dt * self.rotation_rate).rem(2.0 * ::core::f32::consts::PI);
    }

    const fn contains(&self, x: f32) -> bool {
        if self.current_pos > self.previous_pos {
            x >= self.previous_pos && x <= self.current_pos
        } else {
            x <= self.current_pos || x >= self.previous_pos
        }
    }
}

pub fn main() {
    let (mut rl, thread) = raylib::init()
        .msaa_4x()
        .size(800, 600)
        .title("pov-sim")
        .resizable()
        .build();

    const NUM_LED: u32 = 30;
    const HUB_PERC: f32 = 0.2;

    let mut leds = Vec::new();
    for d in 0..72 {
        let angle = (d * 5) as f32 * ::core::f32::consts::PI / 180.0;
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
            });
        }
    }

    let mut state = RotationState {
        current_pos: 0.0,
        previous_pos: 0.0,
        rotation_rate: 5.0,
    };

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

        for l in &mut leds {
            if state.contains(l.offset) {
                l.fade_val = 1.0;
            } else {
                l.fade_val = (l.fade_val - d.get_frame_time()).max(0.0);
            }

            let color = if l.id > 30 { Color::RED } else { Color::BLUE }.alpha(l.fade_val);

            d.draw_circle(
                cx + (l.loc.0 * wheel_inner_radius) as i32,
                cy + (l.loc.1 * wheel_inner_radius) as i32,
                2.0,
                color,
            );
        }

        d.draw_text("Hello, world!", 12, 12, 20, Color::BLACK);
        d.draw_text(&format!("FPS: {}", d.get_fps()), 12, 40, 20, Color::BLACK);
    }
}
