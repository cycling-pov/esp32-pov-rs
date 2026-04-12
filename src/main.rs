use std::{f32, ops::Rem};

use bevy::{
    dev_tools::fps_overlay::{FpsOverlayConfig, FpsOverlayPlugin, FrameTimeGraphConfig},
    ecs::schedule::ExecutorKind,
    input::common_conditions::{input_just_pressed, input_toggle_active},
    log::tracing::instrument,
    prelude::*,
    sprite_render::AlphaMode2d,
    text::TextColor,
    window::WindowTheme,
};
pub mod algorithms;

struct OverlayColor;

impl OverlayColor {
    const RED: Color = Color::srgb(1.0, 0.0, 0.0);
    const GREEN: Color = Color::srgb(0.0, 1.0, 0.0);
}

fn main() {
    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "POV SIM".into(),
                name: Some("povsim.app".into()),
                resolution: (1024, 768).into(),
                fit_canvas_to_parent: true,
                prevent_default_event_handling: false,
                window_theme: Some(WindowTheme::Dark),
                //present_mode: bevy::window::PresentMode::Immediate,
                ..Default::default()
            }),
            ..Default::default()
        }),
        FpsOverlayPlugin {
            config: FpsOverlayConfig {
                text_config: TextFont {
                    font_size: 11.0,
                    ..Default::default()
                },
                refresh_interval: core::time::Duration::from_millis(100),
                enabled: true,
                frame_time_graph_config: FrameTimeGraphConfig {
                    enabled: true,
                    min_fps: 30.0,
                    target_fps: 144.0,
                },
                ..Default::default()
            },
        },
    ))
    .insert_resource(ClearColor(Color::srgb_u8(255, 255, 255)))
    .insert_resource(ThemeState::default())
    .insert_resource(RotationState {
        rotation_rate: 10.0,
        previous_pos: 0.0,
        current_pos: 0.0,
    })
    .add_systems(Startup, setup)
    .edit_schedule(Update, |sched| {
        sched.set_executor_kind(ExecutorKind::SingleThreaded);
    })
    .edit_schedule(PreUpdate, |sched| {
        sched.set_executor_kind(ExecutorKind::SingleThreaded);
    })
    .edit_schedule(PostUpdate, |sched| {
        sched.set_executor_kind(ExecutorKind::SingleThreaded);
    });

    app.add_systems(PostStartup, (set_theme, update_text));
    app.add_systems(
        PreUpdate,
        (toggle_theme, set_theme).run_if(input_just_pressed(KeyCode::KeyT)),
    );
    app.add_systems(
        PreUpdate,
        (rotation_change_input, update_text).run_if(
            input_just_pressed(KeyCode::ArrowUp).or(input_just_pressed(KeyCode::ArrowDown)),
        ),
    );

    app.add_systems(Update, (update_rotation_state, update_pattern));
    app.add_systems(
        PostUpdate,
        update_pattern_meshes.run_if(input_toggle_active(true, KeyCode::KeyU)),
    );

    app.run();
}

fn rotation_change_input(input: Res<ButtonInput<KeyCode>>, mut cmd: ResMut<RotationState>) {
    let dir = if input.just_pressed(KeyCode::ArrowUp) {
        1.0
    } else {
        -1.0
    };
    cmd.rotation_rate = (cmd.rotation_rate + 0.5 * dir).min(10.0).max(0.0);
}

#[derive(Debug, Resource)]
struct RotationState {
    rotation_rate: f32,
    previous_pos: f32,
    current_pos: f32,
}

impl RotationState {
    const fn contains(&self, x: f32) -> bool {
        if self.current_pos > self.previous_pos {
            x >= self.previous_pos && x <= self.current_pos
        } else {
            x <= self.current_pos || x >= self.previous_pos
        }
    }
}

#[derive(Resource)]
struct ThemeState {
    dark_theme: bool,
}

impl Default for ThemeState {
    fn default() -> Self {
        Self { dark_theme: true }
    }
}

#[derive(Component)]
struct LED {
    id: u32,
    offset: f32,
    fade_state: f32,
}

#[derive(Component)]
struct TextStatUpdate;

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.spawn(Camera2d);

    const RADIUS_OUTER: f32 = 300.0;
    const WHEEL_THICKNESS: f32 = 20.0;
    const RADIUS_HUB: f32 = 20.0;

    const WHEEL_COLOR: Color = Color::BLACK;
    const HUB_COLOR: Color = Color::linear_rgb(0.05, 0.05, 0.05);

    commands.spawn((
        Mesh2d(meshes.add(Circle::new(RADIUS_HUB))),
        MeshMaterial2d(materials.add(HUB_COLOR)),
        Transform::from_xyz(0.0, 0.0, 1.0),
    ));

    commands.spawn((
        Mesh2d(meshes.add(Circle::new(RADIUS_OUTER).to_ring(WHEEL_THICKNESS))),
        MeshMaterial2d(materials.add(WHEEL_COLOR)),
        Transform::from_xyz(0.0, 0.0, 1.0),
    ));

    const LED_LEN: f32 = RADIUS_OUTER - RADIUS_HUB - WHEEL_THICKNESS;

    const NUM_LEDS: u32 = 50;
    const LED_RADIUS: f32 = WHEEL_THICKNESS / 10.0;
    const NUM_LED_SPOKES: u32 = 72;
    const DEG_PER_SPOKE: u32 = 360 / NUM_LED_SPOKES;
    assert_eq!(360 % DEG_PER_SPOKE, 0);
    const DEG_PER_SPOKE_F: f32 = DEG_PER_SPOKE as f32;

    const DEG_2_RAD: f32 = f32::consts::PI / 180.0;

    let pxmesh = Mesh2d(meshes.add(Rectangle::new(LED_RADIUS, LED_RADIUS)));

    for deg in 0..NUM_LED_SPOKES {
        let offset = deg as f32 * DEG_PER_SPOKE_F * DEG_2_RAD;
        let (s1, c1) = offset.sin_cos();

        for i in 0..NUM_LEDS {
            let led_rad = RADIUS_HUB + LED_LEN / (NUM_LEDS as f32) * i as f32 + LED_RADIUS;

            commands.spawn((
                pxmesh.clone(),
                MeshMaterial2d(materials.add(ColorMaterial {
                    color: Color::WHITE,
                    alpha_mode: AlphaMode2d::Blend,
                    ..Default::default()
                })),
                Transform::from_xyz(led_rad * c1, led_rad * s1, 5.0),
                LED {
                    id: i,
                    offset,
                    fade_state: 0.0,
                },
            ));
        }
    }

    commands.spawn((
        Text::new(""),
        TextColor(Color::WHITE),
        TextStatUpdate,
        Node {
            position_type: PositionType::Absolute,
            top: px(12),
            left: px(200),
            ..default()
        },
    ));
}

fn update_text(mut query: Query<&mut Text, With<TextStatUpdate>>, cmd: Res<RotationState>) {
    for mut t in &mut query {
        t.0 = format!("Rotation Rate: {:0.2}", cmd.rotation_rate);
    }
}

#[instrument(name = "rotation_state")]
fn update_rotation_state(time: Res<Time>, mut state: ResMut<RotationState>) {
    state.previous_pos = state.current_pos;
    state.current_pos =
        (state.current_pos + time.delta_secs() * state.rotation_rate).rem(2.0 * f32::consts::PI);
}

#[instrument(name = "update_pattern")]
fn update_pattern(mut query: Query<&mut LED>, state: Res<RotationState>, time: Res<Time>) {
    for mut led in &mut query {
        if state.contains(led.offset) {
            led.fade_state = 1.0;
        } else {
            led.fade_state = (led.fade_state - time.delta_secs()).max(0.0);
        }
    }
}

#[instrument(name = "update_pattern_meshes", skip_all)]
fn update_pattern_meshes(
    mut query: Query<(&LED, &MeshMaterial2d<ColorMaterial>)>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    for (led, h) in &mut query {
        let col = if led.id > 30 {
            Color::WHITE
        } else {
            Color::srgb_u8(0, 255, 255)
        }
        .with_alpha(led.fade_state);

        let mat = materials.get_mut(h).unwrap();
        mat.color = col;
    }
}

fn toggle_theme(mut state: ResMut<ThemeState>) {
    state.dark_theme = !state.dark_theme;
}

fn set_theme(
    mut color: ResMut<ClearColor>,
    state: Res<ThemeState>,
    mut text: Query<&mut TextColor>,
) {
    if state.dark_theme {
        color.0 = Color::linear_rgb(0.05, 0.05, 0.1);

        for mut t in &mut text {
            t.0 = Color::WHITE;
        }
    } else {
        color.0 = Color::linear_rgb(0.95, 0.95, 0.95);

        for mut t in &mut text {
            t.0 = Color::BLACK;
        }
    }
}
