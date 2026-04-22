use std::ops::Rem;

use bevy::{
    app::{Plugin, PreUpdate},
    ecs::resource::Resource,
    input::{common_conditions::input_pressed, keyboard::KeyCode},
    prelude::*,
};
use pov_algs::{CIRCLE_RADIANS, DEGREES_TO_RADIANS, angular_error};

pub const NUM_SPOKES: usize = 2;

pub struct RotationPlugin;

#[derive(Resource, Event, Default, Debug, Clone, Copy)]
pub struct RotationSettings {
    pub rate: f32,
    pub fade: f32,
}

impl Plugin for RotationPlugin {
    fn build(&self, app: &mut bevy::app::App) {
        app.insert_resource(RotationSettings {
            rate: 12.0,
            fade: 0.3,
        })
        .insert_resource(RotationState::new(NUM_SPOKES))
        .add_systems(
            PreUpdate,
            (
                rotation_change_input.run_if(
                    input_pressed(KeyCode::ArrowUp)
                        .or(input_pressed(KeyCode::ArrowDown))
                        .or(input_pressed(KeyCode::ArrowLeft))
                        .or(input_pressed(KeyCode::ArrowRight)),
                ),
                update_rotation_state,
            ),
        );
    }
}

#[derive(Debug, Resource)]
pub struct RotationState {
    spoke_positions: Vec<SpokePos>,
}

#[derive(Default, Debug, Clone, Copy)]
pub struct SpokePos {
    pub pos: f32,
    pub prev: f32,
}

impl SpokePos {
    pub const fn has_rotated(&self) -> bool {
        self.prev > self.pos
    }

    pub fn contains(&self, x: f32) -> bool {
        let r1 = if self.pos > self.prev {
            x >= self.prev && x <= self.pos
        } else if self.pos != self.prev {
            x <= self.pos || x >= self.prev
        } else {
            false
        };

        let r2 = angular_error(x - self.pos).abs() < 1.0 * DEGREES_TO_RADIANS;

        r1 || r2
    }
}

impl RotationState {
    fn new(num_spokes: usize) -> Self {
        assert!(num_spokes > 0);
        let mut s = Self {
            spoke_positions: vec![SpokePos::default(); num_spokes],
        };
        s.reset();
        s
    }

    pub fn contains(&self, angle: f32) -> Option<usize> {
        self.spoke_positions
            .iter()
            .enumerate()
            .filter(|(_, x)| x.contains(angle))
            .map(|(i, _)| i)
            .next()
    }

    pub fn num_spokes(&self) -> usize {
        self.spoke_positions.len()
    }

    pub fn reset(&mut self) {
        let offset = self.offset_angle();
        for (i, pos) in self.spoke_positions.iter_mut().enumerate() {
            let current = offset * (i as f32);
            pos.pos = current;
            pos.prev = current;
        }
    }

    pub fn step(&mut self, settings: &RotationSettings, dt: f32) {
        let angle_offset = self.offset_angle();

        let init_angle = if let Some(pos) = self.spoke_positions.first_mut() {
            pos.prev = pos.pos;
            pos.pos = (pos.pos + dt * settings.rate).rem(CIRCLE_RADIANS);
            pos.pos
        } else {
            panic!("unable to get spoke values");
        };

        for pos in &mut self.spoke_positions[1..] {
            pos.prev = pos.pos;
            pos.pos = (init_angle + angle_offset).rem(CIRCLE_RADIANS);
        }
    }

    fn offset_angle(&self) -> f32 {
        let num_spokes = self.spoke_positions.len();
        assert!(num_spokes > 0);
        CIRCLE_RADIANS / num_spokes as f32
    }

    pub fn has_rotated_spoke(&self, spoke: usize) -> bool {
        self.spoke_positions
            .get(spoke)
            .is_some_and(|x| x.has_rotated())
    }

    pub fn position(&self, spoke: usize) -> &SpokePos {
        &self.spoke_positions[spoke]
    }
}

fn update_rotation_state(
    time: Res<Time>,
    mut state: ResMut<RotationState>,
    settings: Res<RotationSettings>,
) {
    state.step(&settings, time.delta_secs());
}

fn rotation_change_input(
    mut commands: Commands,
    input: Res<ButtonInput<KeyCode>>,
    mut settings: ResMut<RotationSettings>,
    time: Res<Time>,
) {
    let speed_dir = if input.pressed(KeyCode::ArrowUp) {
        1.0
    } else if input.pressed(KeyCode::ArrowDown) {
        -1.0
    } else {
        0.0
    };

    let fade_dir = if input.pressed(KeyCode::ArrowRight) {
        1.0
    } else if input.pressed(KeyCode::ArrowLeft) {
        -1.0
    } else {
        0.0
    };

    settings.rate = (settings.rate + 4.0 * speed_dir * time.delta_secs()).clamp(0.0, 20.0);
    settings.fade = (settings.fade + 0.5 * fade_dir * time.delta_secs()).clamp(0.1, 2.0);

    commands.trigger(*settings);
}
