use std::ops::Rem;

use bevy::{
    app::{Plugin, PreUpdate},
    ecs::resource::Resource,
    input::{common_conditions::input_pressed, keyboard::KeyCode},
    prelude::*,
};

pub struct RotationPlugin;

#[derive(Event)]
pub struct RotationSettingsUpdated {
    pub rate: f32,
    pub fade: f32,
}

impl Plugin for RotationPlugin {
    fn build(&self, app: &mut bevy::app::App) {
        app.insert_resource(RotationState::new(2, 12.0, 0.2))
            .add_systems(
                PreUpdate,
                rotation_change_input.run_if(
                    input_pressed(KeyCode::ArrowUp)
                        .or(input_pressed(KeyCode::ArrowDown))
                        .or(input_pressed(KeyCode::ArrowLeft))
                        .or(input_pressed(KeyCode::ArrowRight)),
                ),
            );
    }
}

#[derive(Debug, Resource)]
pub struct RotationState {
    spoke_positions: Vec<SpokePosition>,
    pub rotation_rate: f32,
    pub fade_dt: f32,
}

#[derive(Debug, Default, Clone, Copy)]
struct SpokePosition {
    current_pos: f32,
    previous_pos: f32,
}

impl SpokePosition {
    fn contains(&self, x: f32) -> bool {
        let r1 = if self.current_pos > self.previous_pos {
            x >= self.previous_pos && x <= self.current_pos
        } else if self.current_pos != self.previous_pos {
            x <= self.current_pos || x >= self.previous_pos
        } else {
            false
        };

        let r2 = ((x - self.current_pos + core::f32::consts::PI).rem(core::f32::consts::PI * 2.0)
            - core::f32::consts::PI)
            .abs()
            < 1.0 * core::f32::consts::PI / 180.0;

        r1 || r2
    }

    const fn has_rotated(&self) -> bool {
        self.previous_pos > self.current_pos
    }
}

impl RotationState {
    const FULL_CIRCLE: f32 = 2.0 * ::core::f32::consts::PI;

    pub fn new(num_spokes: usize, init_rate: f32, init_fade: f32) -> Self {
        assert!(num_spokes > 0);
        let mut s = Self {
            rotation_rate: init_rate,
            spoke_positions: vec![SpokePosition::default(); num_spokes],
            fade_dt: init_fade,
        };
        s.reset();
        s
    }

    pub fn reset(&mut self) {
        let offset = self.offset_angle();
        for (i, pos) in self.spoke_positions.iter_mut().enumerate() {
            let current = offset * (i as f32);
            pos.current_pos = current;
            pos.previous_pos = current;
        }
    }

    pub fn step(&mut self, dt: f32) {
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

    pub fn contains(&self, x: f32) -> bool {
        for spoke in &self.spoke_positions {
            if spoke.contains(x) {
                return true;
            }
        }

        false
    }

    pub fn num_spokes(&self) -> usize {
        self.spoke_positions.len()
    }

    pub fn has_rotated(&self) -> bool {
        self.has_rotated_spoke(0)
    }

    pub fn has_rotated_spoke(&self, spoke: usize) -> bool {
        self.spoke_positions
            .get(spoke)
            .map_or(false, |x| x.has_rotated())
    }

    pub fn get_settings(&self) -> RotationSettingsUpdated {
        RotationSettingsUpdated {
            rate: self.rotation_rate,
            fade: self.fade_dt,
        }
    }
}

fn rotation_change_input(
    mut commands: Commands,
    input: Res<ButtonInput<KeyCode>>,
    mut cmd: ResMut<RotationState>,
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

    cmd.rotation_rate = (cmd.rotation_rate + 4.0 * speed_dir * time.delta_secs()).clamp(0.0, 20.0);
    cmd.fade_dt = (cmd.fade_dt + 0.5 * fade_dir * time.delta_secs()).clamp(0.1, 2.0);

    commands.trigger(RotationSettingsUpdated {
        rate: cmd.rotation_rate,
        fade: cmd.fade_dt,
    });
}
