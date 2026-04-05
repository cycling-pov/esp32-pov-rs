use std::time::Duration;

use bevy::{
    input::common_conditions::{input_just_pressed, input_toggle_active},
    prelude::*,
    sprite_render::AlphaMode2d,
    text::TextColor,
    time::common_conditions::on_timer,
    window::WindowTheme,
};

pub mod algorithms;

fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "POV SIM".into(),
            name: Some("povsim.app".into()),
            resolution: (1024, 768).into(),
            fit_canvas_to_parent: true,
            prevent_default_event_handling: false,
            window_theme: Some(WindowTheme::Dark),
            ..Default::default()
        }),
        ..Default::default()
    }))
    .insert_resource(ClearColor(Color::srgb_u8(255, 255, 255)))
    .insert_resource(ThemeState::default())
    .insert_resource(RotationCommand { rotation_rate: 1.0 })
    .add_systems(Startup, setup_new);

    app.add_systems(
        Update,
        rotate_wheel.run_if(input_toggle_active(true, KeyCode::KeyR)),
    );

    app.add_systems(PostStartup, (set_theme, update_text));
    app.add_systems(
        Update,
        (toggle_theme, set_theme).run_if(input_just_pressed(KeyCode::KeyT)),
    );

    app.add_systems(
        Update,
        spawn_led_colors.run_if(input_just_pressed(KeyCode::KeyS)),
    );

    app.add_systems(
        Update,
        spawn_led_colors.run_if(on_timer(Duration::from_millis(10))),
    );

    app.add_systems(
        PreUpdate,
        (rotation_increase, update_text).run_if(input_just_pressed(KeyCode::ArrowUp)),
    );
    app.add_systems(
        PreUpdate,
        (rotation_decrease, update_text).run_if(input_just_pressed(KeyCode::ArrowDown)),
    );

    app.add_systems(Update, fade_lights);
    app.add_systems(PostUpdate, delete_lights);

    app.run();
}

fn rotation_increase(mut cmd: ResMut<RotationCommand>) {
    cmd.rotation_rate = (cmd.rotation_rate + 0.25).min(10.0);
}

fn rotation_decrease(mut cmd: ResMut<RotationCommand>) {
    cmd.rotation_rate = (cmd.rotation_rate - 0.25).max(0.0);
}

#[derive(Resource)]
struct RotationCommand {
    rotation_rate: f32,
}

#[derive(Component)]
struct Rotator;

#[derive(Resource)]
struct ThemeState {
    dark_theme: bool,
}

#[derive(Component)]
struct LED {
    id: u32,
    offset: f32,
}

#[derive(Component)]
struct LEDInstance {
    fade_val: f32,
}

#[derive(Component)]
struct TextStatUpdate;

impl Default for LEDInstance {
    fn default() -> Self {
        Self { fade_val: 1.0 }
    }
}

impl Default for ThemeState {
    fn default() -> Self {
        Self { dark_theme: true }
    }
}

fn setup_new(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.spawn(Camera2d);

    const RADIUS_OUTER: f32 = 300.0;
    const WHEEL_THICKNESS: f32 = 20.0;
    const RADIUS_HUB: f32 = 20.0;
    const NUM_SPOKES: u32 = 18;

    const WHEEL_COLOR: Color = Color::BLACK;
    const SPOKE_COLOR: Color = Color::linear_rgb(0.3, 0.3, 0.3);
    const HUB_COLOR: Color = Color::linear_rgb(0.05, 0.05, 0.05);

    const NUM_LEDS: u32 = 50;

    let root = commands
        .spawn((
            Mesh2d(meshes.add(Circle::new(RADIUS_HUB))),
            MeshMaterial2d(materials.add(HUB_COLOR)),
            Transform::from_xyz(0.0, 0.0, 1.0),
            Rotator,
        ))
        .id();

    assert_eq!(360 % NUM_SPOKES, 0);
    const ANGLE_SPACING: u32 = 360 / NUM_SPOKES;

    for i in 0..NUM_SPOKES {
        let (s, c) = ((ANGLE_SPACING * i) as f32 * core::f32::consts::PI / 180.0).sin_cos();
        let dir_vec = Vec2::new(c, s);
        commands.spawn((
            Mesh2d(meshes.add(Segment2d::new(
                Vec2::ONE * RADIUS_HUB * dir_vec,
                Vec2::ONE * (RADIUS_OUTER - WHEEL_THICKNESS / 2.0) * dir_vec,
            ))),
            MeshMaterial2d(materials.add(SPOKE_COLOR)),
            Transform::from_xyz(0.0, 0.0, -1.0),
            ChildOf(root),
        ));
    }

    commands.spawn((
        Mesh2d(meshes.add(Circle::new(RADIUS_OUTER).to_ring(WHEEL_THICKNESS))),
        MeshMaterial2d(materials.add(WHEEL_COLOR)),
        Transform::from_xyz(0.0, 0.0, 1.0),
        ChildOf(root),
    ));

    const RECT_WIDTH: f32 = RADIUS_OUTER - RADIUS_HUB - WHEEL_THICKNESS;

    commands.spawn((
        Mesh2d(meshes.add(Rectangle::new(RECT_WIDTH, WHEEL_THICKNESS / 2.0))),
        MeshMaterial2d(materials.add(SPOKE_COLOR)),
        Transform::from_xyz(RADIUS_HUB + RECT_WIDTH / 2.0, 0.0, 0.0),
        ChildOf(root),
    ));
    commands.spawn((
        Mesh2d(meshes.add(Rectangle::new(RECT_WIDTH, WHEEL_THICKNESS / 2.0))),
        MeshMaterial2d(materials.add(SPOKE_COLOR)),
        Transform::from_xyz(-RADIUS_HUB - RECT_WIDTH / 2.0, 0.0, 0.0),
        ChildOf(root),
    ));

    const LED_RADIUS: f32 = WHEEL_THICKNESS / 10.0;

    for i in 0..NUM_LEDS {
        let xval = RADIUS_HUB + RECT_WIDTH / (NUM_LEDS as f32) * i as f32 + LED_RADIUS;

        commands.spawn((
            Mesh2d(meshes.add(Circle::new(LED_RADIUS))),
            MeshMaterial2d(materials.add(Color::WHITE)),
            Transform::from_xyz(xval, 0.0, 5.0),
            LED { id: i, offset: 0.0 },
            ChildOf(root),
        ));

        commands.spawn((
            Mesh2d(meshes.add(Circle::new(LED_RADIUS))),
            MeshMaterial2d(materials.add(Color::WHITE)),
            Transform::from_xyz(-xval, 0.0, 5.0),
            GlobalTransform::default(),
            LED {
                id: i,
                offset: std::f32::consts::PI,
            },
            ChildOf(root),
        ));
    }

    let text = "Press 'R' to pause/resume rotation\nPress 'T' to toggle theme".to_string();

    commands.spawn((
        Text::new(text),
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: px(12),
            left: px(12),
            ..default()
        },
    ));

    commands.spawn((
        Text::new("Rotation Rate: {}"),
        TextColor(Color::WHITE),
        TextStatUpdate,
        Node {
            position_type: PositionType::Absolute,
            top: px(100),
            left: px(12),
            ..default()
        },
    ));
}

fn update_text(mut query: Query<&mut Text, With<TextStatUpdate>>, cmd: Res<RotationCommand>) {
    for mut t in &mut query {
        t.0 = format!("Rotation Rate: {:0.2}", cmd.rotation_rate);
    }
}

fn rotate_wheel(
    mut query: Query<&mut Transform, With<Rotator>>,
    time: Res<Time>,
    cmd: Res<RotationCommand>,
) {
    for mut transform in &mut query {
        transform.rotate_z(time.delta_secs() * cmd.rotation_rate);
    }
}

fn spawn_led_colors(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    query: Query<(&LED, &GlobalTransform)>,
) {
    for (led, tr) in &query {
        commands.spawn((
            Mesh2d(meshes.add(Circle::new(1.0))),
            MeshMaterial2d(materials.add(ColorMaterial {
                color: Color::WHITE,
                alpha_mode: AlphaMode2d::Blend,
                ..Default::default()
            })),
            tr.compute_transform(),
            LEDInstance::default(),
        ));
    }
}

fn fade_lights(
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut query: Query<(&mut LEDInstance, &MeshMaterial2d<ColorMaterial>)>,
    time: Res<Time>,
) {
    fn color_with_alpha(col: &Color, alpha: f32) -> Color {
        let prev = col.to_linear();
        Color::linear_rgba(prev.red, prev.green, prev.blue, alpha.max(0.0))
    }

    for (mut l, h) in &mut query {
        l.fade_val -= time.delta_secs();
        let color = materials.get_mut(h).unwrap();
        color.color = color_with_alpha(&color.color, l.fade_val);
    }
}

fn delete_lights(mut commands: Commands, mut query: Query<(Entity, &LEDInstance)>) {
    for (e, l) in &mut query {
        if l.fade_val <= 0.0 {
            commands.entity(e).despawn();
        }
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
