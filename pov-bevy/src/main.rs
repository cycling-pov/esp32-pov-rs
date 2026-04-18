mod images;
mod state;

use bevy::{
    asset::RenderAssetUsages,
    dev_tools::fps_overlay::{FpsOverlayConfig, FpsOverlayPlugin, FrameTimeGraphConfig},
    input::common_conditions::input_just_pressed,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    text::TextColor,
    window::WindowTheme,
};

use crate::{
    images::{ImageChanged, ImageState},
    state::{RotationPlugin, RotationSettingsChanged, RotationState},
};

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
                    target_fps: 60.0,
                },
                ..Default::default()
            },
        },
    ))
    .insert_resource(ImageState::default())
    .insert_resource(ClearColor(Color::srgb_u8(255, 255, 255)))
    .insert_resource(ThemeState::default())
    .add_systems(Startup, setup)
    .add_plugins(RotationPlugin);

    app.add_systems(PostStartup, set_theme);
    app.add_systems(
        PreUpdate,
        (toggle_theme, set_theme).run_if(input_just_pressed(KeyCode::KeyT)),
    );
    app.add_systems(
        PreUpdate,
        set_next_image.run_if(input_just_pressed(KeyCode::KeyA)),
    );
    app.add_observer(update_text);
    app.add_observer(update_text_image);

    app.add_systems(Update, (update_rotation_state, update_pattern));

    app.run();
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
struct Led {
    //id: u32,
    offset: f32,
    //radius_perc: f32,
    loc: (f32, f32),
    fade: f32,
}

#[derive(Component)]
struct TextStatUpdate;

#[derive(Component)]
struct TextImageNameUpdate;

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut images: ResMut<Assets<Image>>,
    state: Res<RotationState>,
    image_state: Res<ImageState>,
) {
    commands.spawn(Camera2d);

    const RADIUS_OUTER: f32 = 300.0;
    const WHEEL_THICKNESS: f32 = 20.0;
    const RADIUS_HUB: f32 = 20.0;

    const WHEEL_COLOR: Color = Color::BLACK;
    const HUB_COLOR: Color = Color::linear_rgb(0.05, 0.05, 0.05);

    let hub = commands
        .spawn((
            Mesh2d(meshes.add(Circle::new(RADIUS_HUB))),
            MeshMaterial2d(materials.add(HUB_COLOR)),
            Transform::from_xyz(0.0, 0.0, 1.0),
        ))
        .id();

    commands.spawn((
        Mesh2d(meshes.add(Circle::new(RADIUS_OUTER).to_ring(WHEEL_THICKNESS))),
        MeshMaterial2d(materials.add(WHEEL_COLOR)),
        Transform::from_xyz(0.0, 0.0, 1.0),
        ChildOf(hub),
    ));

    const LED_LEN: f32 = RADIUS_OUTER - RADIUS_HUB - WHEEL_THICKNESS;

    const NUM_LED: u32 = 40;
    const HUB_PERC: f32 = 0.2;
    const NUM_LED_SPOKES: u32 = 72 * 2;

    let circle_img = images.add(create_circle_image(8));

    for d in 0..NUM_LED_SPOKES {
        let angle = (d as f32 * 360.0 / NUM_LED_SPOKES as f32) * ::core::f32::consts::PI / 180.0;
        let (s, c) = angle.sin_cos();

        for i in 0..NUM_LED {
            let radius_perc = i as f32 / NUM_LED as f32;
            let radius_mod = radius_perc.powf(0.8);

            let radius = HUB_PERC + (1.0 - HUB_PERC) * radius_mod;
            let radius_val = (LED_LEN + RADIUS_HUB) * radius;
            commands.spawn((
                Sprite {
                    image: circle_img.clone(),
                    color: Color::WHITE,
                    custom_size: Some(Vec2::splat(5.0)),
                    ..default()
                },
                Transform::from_xyz(radius_val * c, -radius_val * s, 1.0),
                Led {
                    //id: i,
                    fade: 1.0,
                    offset: angle,
                    //radius_perc: radius,
                    loc: (radius * c, radius * s),
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
            left: px(12),
            ..default()
        },
    ));

    commands.spawn((
        Text::new(""),
        TextColor(Color::WHITE),
        TextImageNameUpdate,
        Node {
            position_type: PositionType::Absolute,
            top: px(64),
            left: px(12),
            ..default()
        },
    ));

    commands.trigger(state.get_settings());
    commands.trigger(ImageChanged {
        name: image_state.current_name().into(),
    });
}

/// Creates a small white circle image for use as a sprite texture.
/// All trail sprites share this single GPU texture, enabling sprite batching
/// (one draw call for all ~7000 active dots instead of one per entity).
fn create_circle_image(size: u32) -> Image {
    let mut data = vec![0u8; (size * size * 4) as usize];
    let center = (size as f32 - 1.0) / 2.0;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let idx = ((y * size + x) * 4) as usize;
            data[idx] = 255;
            data[idx + 1] = 255;
            data[idx + 2] = 255;
            data[idx + 3] = if (dx * dx + dy * dy).sqrt() <= center {
                255
            } else {
                0
            };
        }
    }
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn update_text(
    event: On<RotationSettingsChanged>,
    mut query: Query<&mut Text, With<TextStatUpdate>>,
) {
    let e = event.event();
    for mut t in &mut query {
        t.0 = format!("Rotation Rate: {:0.2}\nFade Time: {:0.2}", e.rate, e.fade);
    }
}

fn update_text_image(
    event: On<ImageChanged>,
    mut query: Query<&mut Text, With<TextImageNameUpdate>>,
) {
    let e = event.event();
    for mut t in &mut query {
        t.0 = format!("Image: {}", &e.name);
    }
}

fn set_next_image(mut commands: Commands, mut images: ResMut<ImageState>) {
    images.next_img();
    commands.trigger(ImageChanged {
        name: images.current_name().into(),
    });
}

fn update_rotation_state(
    time: Res<Time>,
    mut state: ResMut<RotationState>,
    mut images: ResMut<ImageState>,
) {
    state.step(time.delta_secs());

    images.step_dt(time.delta_secs());
    if state.has_rotated() {
        images.step_rotation();
    }
}

fn update_pattern(
    mut query: Query<(&mut Led, &mut Sprite)>,
    state: Res<RotationState>,
    time: Res<Time>,
    images: Res<ImageState>,
) {
    let img = images.current_image();

    for (mut led, mut sprite) in &mut query {
        if state.contains(led.offset) {
            led.fade = 1.0;

            let px = img.get_nearest(led.loc.0, led.loc.1);
            sprite.color = Color::srgb_u8(px.red, px.green, px.blue);
        } else {
            led.fade = (led.fade - time.delta_secs() / state.fade_dt).max(0.0);
            sprite.color = sprite.color.with_alpha(led.fade);
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
        color.0 = Color::srgb_u8(169, 169, 169);

        for mut t in &mut text {
            t.0 = Color::BLACK;
        }
    }
}
