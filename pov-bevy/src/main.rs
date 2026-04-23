mod estimator;
mod images;
mod state;
mod theme;

use std::{f32, ops::Rem};

use bevy::{
    asset::RenderAssetUsages,
    dev_tools::fps_overlay::{FpsOverlayConfig, FpsOverlayPlugin, FrameTimeGraphConfig},
    input::common_conditions::input_just_pressed,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    text::TextColor,
    window::WindowTheme,
};
use pov_algs::{CIRCLE_RADIANS, DEGREES_TO_RADIANS, LedGeometry, angular_error};
use pov_images::DEFAULT_LEDS;

use crate::{
    estimator::PositionEstimator,
    images::{ImageChanged, ImageState},
    state::{NUM_SPOKES, RotationPlugin, RotationSettings, RotationState},
    theme::{ThemePlugin, ThemeState},
};

fn main() {
    let geometry = SimGeometry::new(NUM_SPOKES, DEFAULT_LEDS);

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
    .insert_resource(ImageState::new(&geometry))
    .insert_resource(PositionEstimator::default())
    .insert_resource(ClearColor(Color::srgb_u8(255, 255, 255)))
    .insert_resource(geometry)
    .add_systems(Startup, setup)
    .add_systems(Update, update_estimator_text)
    .add_plugins(RotationPlugin)
    .add_plugins(ThemePlugin)
    .add_observer(set_theme);

    app.add_systems(
        PreUpdate,
        set_next_image.run_if(input_just_pressed(KeyCode::KeyA)),
    );

    app.add_observer(update_text);
    app.add_observer(update_text_image);

    app.add_systems(Update, (update_estimator, update_pattern));

    app.run();
}

#[derive(Component)]
struct Led {
    id: usize,
    angle: f32,
    fade: f32,
}

#[derive(Component)]
struct TextStatUpdate;

#[derive(Component)]
struct TextImageNameUpdate;

#[derive(Component)]
struct TextEstimatorUpdate;

#[derive(Resource)]
struct SimGeometry {
    num_spokes: usize,
    radii: Vec<f32>,
    hub_perc: f32,
    wheel_radius: f32,
    wheel_thickness: f32,
}

impl SimGeometry {
    pub fn new(num_spokes: usize, num_leds: usize) -> Self {
        let hub_perc = 0.2;

        let radii = {
            let mut radii = vec![0.0f32; num_leds];

            for (i, r) in radii.iter_mut().enumerate() {
                let linear_percentage = i as f32 / num_leds as f32;
                let modified_percentage = linear_percentage.powf(0.8);

                *r = hub_perc + (1.0 - hub_perc) * modified_percentage;
            }

            radii
        };

        Self {
            num_spokes,
            radii,
            hub_perc,
            wheel_radius: 300.0,
            wheel_thickness: 20.0,
        }
    }
}

impl LedGeometry for SimGeometry {
    fn led_unit_positions(&self) -> &[f32] {
        &self.radii
    }

    fn num_spokes(&self) -> usize {
        self.num_spokes
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut images: ResMut<Assets<Image>>,
    geometry: Res<SimGeometry>,
    image_state: Res<ImageState>,
    settings: Res<RotationSettings>,
) {
    commands.spawn(Camera2d);
    let radius_hub = (geometry.wheel_radius - geometry.wheel_thickness) * geometry.hub_perc * 0.8;

    const WHEEL_COLOR: Color = Color::BLACK;
    const HUB_COLOR: Color = Color::linear_rgb(0.05, 0.05, 0.05);

    // Creates a small white circle image for use as a sprite texture.
    // All trail sprites share this single GPU texture, enabling sprite batching
    let circle_img_size = 8;
    let circle_img = images.add({
        let bytes_per_px = 4;
        let mut data = vec![0u8; (circle_img_size * circle_img_size * bytes_per_px) as usize];
        let center = (circle_img_size as f32 - 1.0) / 2.0;
        for y in 0..circle_img_size {
            for x in 0..circle_img_size {
                let dx = x as f32 - center;
                let dy = y as f32 - center;
                let idx = ((y * circle_img_size + x) * bytes_per_px) as usize;
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
                width: circle_img_size,
                height: circle_img_size,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        )
    });

    // Setup the root hub for the wheel, followed by the wheel tyre
    let hub = commands
        .spawn((
            Mesh2d(meshes.add(Circle::new(radius_hub))),
            MeshMaterial2d(materials.add(HUB_COLOR)),
            Transform::from_xyz(0.0, 0.0, 1.0),
        ))
        .id();

    commands.spawn((
        Mesh2d(meshes.add(Circle::new(geometry.wheel_radius).to_ring(geometry.wheel_thickness))),
        MeshMaterial2d(materials.add(WHEEL_COLOR)),
        Transform::from_xyz(0.0, 0.0, 1.0),
        ChildOf(hub),
    ));

    // Define the overall length of the strip for the LED values
    let led_len: f32 = geometry.wheel_radius - radius_hub - geometry.wheel_thickness;

    // The number of LED spokes to use in the simulation
    const NUM_LED_SPOKES: usize = 144 * 2;

    // Spawn the elements required for each virtual LED spoke
    for d in 0..NUM_LED_SPOKES {
        let angle = (d as f32 * 360.0 / NUM_LED_SPOKES as f32) * DEGREES_TO_RADIANS;
        let (s, c) = angle.sin_cos();

        for (i, r) in geometry.radii.iter().enumerate() {
            let radius_val = (led_len + radius_hub) * r;
            commands.spawn((
                Sprite {
                    image: circle_img.clone(),
                    color: Color::WHITE,
                    custom_size: Some(Vec2::splat(5.0)),
                    ..default()
                },
                Transform::from_xyz(radius_val * c, -radius_val * s, 1.0),
                Led {
                    id: i,
                    fade: 1.0,
                    angle,
                },
            ));
        }
    }

    // Add text fields
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

    commands.spawn((
        Text::new(""),
        TextColor(Color::WHITE),
        TextEstimatorUpdate,
        Node {
            position_type: PositionType::Absolute,
            top: px(96),
            left: px(12),
            ..default()
        },
    ));

    // Trigger commands for updates based on the settings and current image selection
    commands.trigger(*settings);
    commands.trigger(ImageChanged {
        name: image_state.current_name().into(),
    });
}

fn update_text(event: On<RotationSettings>, mut query: Query<&mut Text, With<TextStatUpdate>>) {
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

fn update_estimator(
    time: Res<Time>,
    state: ResMut<RotationState>,
    mut images: ResMut<ImageState>,
    mut estimator: ResMut<PositionEstimator>,
) {
    let spoke_tick = (0..state.num_spokes()).find(|i| state.has_rotated_spoke(*i));
    estimator.step(time.delta_secs(), spoke_tick);

    images.step_dt(time.delta_secs());
    if estimator.has_rotated() {
        images.step_rotation();
    }
}

fn update_estimator_text(
    mut query: Query<&mut Text, With<TextEstimatorUpdate>>,
    estimator: Res<PositionEstimator>,
    state: Res<RotationState>,
) {
    for mut t in &mut query {
        t.0 = format!(
            "Est: {:0.1} ({:0.1}), {:0.2} rad/s",
            estimator.pos.get_current_pos(),
            estimator.pos.get_current_pos() - state.position(0).pos,
            estimator.pos.get_current_rate()
        );
    }
}

fn update_pattern(
    mut query: Query<(&mut Led, &mut Sprite)>,
    state: Res<RotationState>,
    estimator: Res<PositionEstimator>,
    settings: Res<RotationSettings>,
    //geometry: Res<SimGeometry>,
    time: Res<Time>,
    images: Res<ImageState>,
) {
    //let img = images.current_image();
    let img = images.current_image();

    for (mut led, mut sprite) in &mut query {
        if let Some(spoke) = state.contains(led.angle) {
            led.fade = 1.0;

            let pos = state.position(spoke);

            // Assume linear interpolation between states from the absolute position
            let angular_distance = angular_error(pos.pos - pos.prev).abs();
            let percentage_through_arc = if angular_distance > 1e-3 {
                angular_error(led.angle - pos.prev) / angular_distance
            } else {
                1.0
            };

            // Determine the corresponding estimated spoke position value
            let est_pos = estimator.get_spoke(spoke);
            let est_dist = (est_pos.pos - est_pos.prev).abs();

            // Determine the calculated position based on linear interpolation of the state estimate
            let calc_pos = (est_pos.prev + est_dist * percentage_through_arc).rem(CIRCLE_RADIANS);

            // Compute the resulting pixel value from the calculated position
            let px = img.get_pixel(calc_pos, led.id);
            sprite.color = Color::srgb_u8(px.red, px.green, px.blue);
        } else {
            // Default to fading the current color with the given fade time
            led.fade = (led.fade - time.delta_secs() / settings.fade).max(0.0);
            sprite.color = sprite.color.with_alpha(led.fade);
        }
    }
}

fn set_theme(
    event: On<ThemeState>,
    mut color: ResMut<ClearColor>,
    mut text: Query<&mut TextColor>,
) {
    let state = event.event();

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
