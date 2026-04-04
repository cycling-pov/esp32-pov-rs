use bevy::{
    input::common_conditions::{input_just_pressed, input_toggle_active},
    prelude::*,
    text::TextColor,
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
    .add_systems(Startup, setup_new);

    app.add_systems(
        Update,
        rotate_wheel.run_if(input_toggle_active(true, KeyCode::KeyR)),
    );

    app.add_systems(PostStartup, set_theme);
    app.add_systems(
        Update,
        (toggle_theme, set_theme).run_if(input_just_pressed(KeyCode::KeyT)),
    );

    app.run();
}

#[derive(Component)]
struct Rotator;

#[derive(Resource)]
struct ThemeState {
    dark_theme: bool,
}

#[derive(Component)]
struct LED;

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
            LED,
            ChildOf(root),
        ));

        commands.spawn((
            Mesh2d(meshes.add(Circle::new(LED_RADIUS))),
            MeshMaterial2d(materials.add(Color::WHITE)),
            Transform::from_xyz(-xval, 0.0, 5.0),
            LED,
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
}

fn rotate_wheel(mut query: Query<&mut Transform, With<Rotator>>, time: Res<Time>) {
    for mut transform in &mut query {
        transform.rotate_z(time.delta_secs() / 2.0);
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
