//! The Bevy side of the Skate 3 physics: the schedule and the resources. The simulation itself
//! (`GamePhysics`, `SkaterRuntime`, `PlayerControls`, the per-tick pipeline) is `skate3_sim`'s;
//! everything it exports is re-exported here so the old `crate::physics::...` paths keep working.
//!
//! The four sim structs the app keeps as separate resources are wrapped in same-named newtypes
//! (`Deref`/`DerefMut` to the sim's struct): the sim has no Bevy to derive `Resource` with. This is
//! a transitional shim until the engine adapter (docs/skate3-sim-split.md S5) holds one `Sim`.
use crate::{
    app::{FrameSet, SimulationSet},
    world::PlayerRoot,
};
use bevy::prelude::*;
#[allow(unused_imports)]
pub(crate) use skate3_sim::physics::*;

#[derive(Resource, Deref, DerefMut)]
pub(crate) struct GamePhysics(pub skate3_sim::physics::GamePhysics);

#[derive(Resource, Deref, DerefMut)]
pub(crate) struct SkaterRuntime(pub skate3_sim::physics::SkaterRuntime);

#[derive(Resource, Deref, DerefMut)]
pub(crate) struct PlayerControls(pub skate3_sim::physics::PlayerControls);

impl PlayerControls {
    pub fn load(root: &std::path::Path) -> Result<Self, String> {
        skate3_sim::physics::PlayerControls::load(root).map(Self)
    }
}

#[derive(Resource, Default, Deref, DerefMut)]
struct AudioObservationCursor(skate3_sim::physics::AudioObservationCursor);

/// The view the off-board stick is relative to when it isn't the game
/// camera's: in the headset the player sees the skater with their own eyes
/// (the game camera isn't drawn), so "up" on the stick has to be away from
/// them. The native presentation basis (right, up, at) of that view.
#[derive(Resource, Default)]
pub(crate) struct OffboardView(pub Option<[[f32; 3]; 3]>);

pub(crate) struct PhysicsPlugin;
impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut App) {
        let period = app.world().resource::<GamePhysics>().period();
        let asset_root = app
            .world()
            .resource::<crate::config::Config>()
            .asset_root
            .clone();
        let controls = PlayerControls::load(&asset_root)
            .unwrap_or_else(|error| panic!("Cannot initialize trick gesture recognizers: {error}"));
        app.insert_resource(Time::<Fixed>::from_duration(period))
            .insert_resource(controls)
            .add_systems(FixedUpdate, sample.in_set(SimulationSet::Controls))
            .add_systems(FixedUpdate, advance.in_set(SimulationSet::Physics))
            .init_resource::<AudioObservationCursor>()
            .add_systems(
                FixedUpdate,
                publish_audio
                    .after(SimulationSet::Physics)
                    .in_set(crate::app::SKATE3),
            )
            .add_systems(Update, present.in_set(FrameSet::Physics))
            .add_systems(
                PreUpdate,
                preferences
                    .before(crate::input::poll_controllers)
                    .in_set(crate::app::SKATE3),
            );
    }
}

/// The skater's gameplay preferences, every frame as the customiser's profile did before it went (legacy pass 6):
/// now always the default profile's (trucks and wheels 0.7, stance 1, style and posture 0, the stock gestures).
/// Choosing them comes back with character customisation, through the engine contract.
fn preferences(mut physics: ResMut<GamePhysics>, mut skater: ResMut<SkaterRuntime>) {
    static DEFAULT: std::sync::LazyLock<serde_json::Value> =
        std::sync::LazyLock::new(|| serde_json::json!({"selections": {}, "morphs": {}}));
    skate3_sim::physics::apply_preferences(&DEFAULT, &mut physics, &mut skater.animation);
}

/// `SimulationSet::Controls`: the sim's controller sampling on this tick's published actions.
fn sample(
    input: Res<crate::input::PublishedTickInput>,
    physics: Res<GamePhysics>,
    skater: Res<SkaterRuntime>,
    camera: Res<crate::camera::CameraRuntime>,
    view: Option<Res<OffboardView>>,
    mut player: ResMut<PlayerControls>,
) {
    skate3_sim::physics::controls::sample(
        &input.0,
        &physics,
        &skater,
        &camera,
        view.and_then(|v| v.0),
        &mut player,
    )
    .unwrap_or_else(|error| panic!("Offboard controller publication: {error}"));
}

/// `SimulationSet::Physics`: one sim tick. The sim sets its own cadence (camera slow-mo changes
/// the clock), so the fixed timestep is reapplied from it after every tick.
fn advance(
    mut physics: ResMut<GamePhysics>,
    mut skater: ResMut<SkaterRuntime>,
    mut controls: ResMut<PlayerControls>,
    graphs: Res<crate::graph_runtime::StockGraphs>,
    input: Res<crate::input::PublishedTickInput>,
    mut camera: ResMut<crate::camera::CameraRuntime>,
    mut cadence: ResMut<Time<Fixed>>,
    mut exit: MessageWriter<AppExit>,
) {
    if physics.failed {
        return;
    }
    if let Err(message) = skate3_sim::physics::advance(
        &mut physics,
        &mut skater,
        &mut controls,
        &graphs,
        &input.0,
        &mut camera,
    ) {
        error!("{message}");
        exit.write(AppExit::error());
    }
    cadence.set_timestep(physics.period());
}

/// After `SimulationSet::Physics`: the completed tick's player-audio observation, once.
fn publish_audio(
    physics: Res<GamePhysics>,
    skater: Res<SkaterRuntime>,
    camera: Res<crate::camera::CameraRuntime>,
    mut cursor: ResMut<AudioObservationCursor>,
    mut observations: MessageWriter<crate::skate_audio::PlayerAudioObservation>,
) {
    if let Some(observation) =
        skate3_sim::physics::audio_observation::observe(&physics, &skater, &camera, &mut cursor)
    {
        observations.write(crate::skate_audio::PlayerAudioObservation(observation));
    }
}

fn present(
    physics: Res<GamePhysics>,
    history: Res<crate::presentation::Presentation>,
    replay: Res<crate::replay::Replay>,
    time: Res<Time<Fixed>>,
    hold: Res<crate::app::Hold>,
    mut roots: Query<&mut Transform, With<PlayerRoot>>,
) {
    if physics.failed {
        return;
    }
    let Some((previous, current, alpha)) = history.view(&replay, hold.alpha(&time)) else {
        return;
    };
    for mut root in &mut roots {
        *root = crate::presentation::blend(previous.root, current.root, alpha);
    }
}
