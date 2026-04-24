use bevy::{app::Plugin, input::common_conditions::input_just_pressed, prelude::*};

pub struct ThemePlugin;

impl Plugin for ThemePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ThemeState::default())
            .add_systems(PostStartup, init_theme)
            .add_systems(
                PreUpdate,
                toggle_theme.run_if(input_just_pressed(KeyCode::KeyT)),
            );
    }
}

#[derive(Resource, Event, Clone, Copy)]
pub struct ThemeState {
    pub dark_theme: bool,
}

impl Default for ThemeState {
    fn default() -> Self {
        Self { dark_theme: true }
    }
}

fn init_theme(mut commands: Commands, state: Res<ThemeState>) {
    commands.trigger(*state);
}

fn toggle_theme(mut commands: Commands, mut state: ResMut<ThemeState>) {
    state.dark_theme = !state.dark_theme;
    commands.trigger(*state);
}
