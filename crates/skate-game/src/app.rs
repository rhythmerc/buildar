use crate::{
    animation, assets, camera,
    config::Config,
    graph_runtime::StockGraphs,
    input,
    physics::{GamePhysics, PhysicsPlugin, SkaterRuntime},
    verification, world,
};
use crate::engines::{Engine, EngineSet};
use bevy::{
    prelude::*,
    render::{
        RenderPlugin,
        settings::{Backends, InstanceFlags, RenderCreation, WgpuSettings},
    },
};
use skate_data::GameAssets;

/// Skate 3's own per-frame systems. In the headset it is one engine of three (`crate::engines`): what is in this
/// set runs only while Skate 3 is the running engine, and what those systems spawn is hidden while another runs.
/// On the desktop it is the only engine and the set has no condition. Its plugins put their systems here, directly
/// or through their own sets ([`FrameSet`], [`SimulationSet`], ...: see [`add_retail`]); what they add outside it
/// (a plugin of Bevy's) stays shared.
pub(crate) const SKATE3: EngineSet = EngineSet(Engine::Skate3);

/// Skate 3 held while the shared layer has the player busy in the headset (a menu or build mode open:
/// [`engine_api::PadHeld::pause`], read by the XR adapter). Its fixed tick doesn't run, its sounds hold, and the
/// skater and camera are drawn where they were (the presentation's blend is frozen where it was when it paused).
///
/// Bevy's fixed clock itself keeps running: `Time<Virtual>` isn't paused, because the shared layer (menus, build
/// mode) runs on it. Nothing else uses `FixedUpdate`, so its steps just pass with Skate 3's systems skipped,
/// nothing is owed when it resumes, and the first frame back steps as any frame does (no dt spike, no burst of
/// catch-up ticks).
#[derive(Resource, Default, Debug)]
pub(crate) struct Hold {
    paused: bool,
    /// The blend between the last two fixed ticks' snapshots when it paused.
    alpha: f32,
}

impl Hold {
    // (only the headset pauses it this way)
    #[cfg_attr(not(any(feature = "xr", test)), allow(dead_code))]
    pub(crate) fn set(&mut self, paused: bool, fixed: &Time<Fixed>) {
        if paused && !self.paused {
            self.alpha = fixed.overstep_fraction();
        }
        self.paused = paused;
    }

    pub(crate) fn paused(&self) -> bool {
        self.paused
    }

    /// How far to blend from the previous fixed tick's snapshot to the last one: the fixed clock's overstep, or
    /// where it was when Skate 3 paused.
    pub(crate) fn alpha(&self, fixed: &Time<Fixed>) -> f32 {
        if self.paused { self.alpha } else { fixed.overstep_fraction() }
    }
}

/// The run condition of Skate 3's fixed tick.
fn ticking(hold: Option<Res<Hold>>) -> bool {
    hold.is_none_or(|h| !h.paused)
}

/// [`Hold`]: everything Skate 3 runs in `FixedUpdate` (all of it is in [`SKATE3`]) waits while it is paused.
fn hold(app: &mut App) {
    app.init_resource::<Hold>()
        .configure_sets(FixedUpdate, SKATE3.run_if(ticking));
}

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) enum FrameSet {
    Assets,
    Physics,
    Animation,
    Verification,
}

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) enum SimulationSet {
    Input,
    Controls,
    Physics,
}

pub(crate) fn build(
    config: Config,
    manifest: GameAssets,
    graphs: StockGraphs,
    physics: GamePhysics,
    skater: SkaterRuntime,
) -> App {
    let retail_scene = config
        .map
        .as_ref()
        .is_some_and(|map| crate::retail_render::RetailScene::for_map(map));
    let mut app = App::new();
    crate::custom_models::register_source(&mut app);
    app.register_asset_source(
        "mods",
        bevy::asset::io::AssetSourceBuilder::platform_default(
            &crate::modding::package_root().to_string_lossy(),
            None,
        ),
    );
    // Bevy's Android default `AssetReader` only ever reads from inside the
    // APK's bundled `assets/` (via `AssetManager`); it ignores
    // `AssetPlugin::file_path` entirely. The Quest build instead pushes a
    // converted installation to the app's external files directory (see
    // scripts/quest/push-assets.sh), so the default source needs a plain
    // filesystem reader pointed there, registered before `AssetPlugin`
    // builds its own (platform) default.
    #[cfg(target_os = "android")]
    {
        let path = config.asset_root.clone();
        app.register_asset_source(
            bevy::asset::io::AssetSourceId::Default,
            bevy::asset::io::AssetSourceBuilder::new(move || {
                Box::new(bevy::asset::io::file::FileAssetReader::new(path.clone()))
                    as Box<dyn bevy::asset::io::ErasedAssetReader>
            }),
        );
    }
    let plugins = DefaultPlugins
            .set(AssetPlugin {
                file_path: config.asset_root.to_string_lossy().into_owned(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: config
                        .multiplayer
                        .title
                        .clone()
                        .unwrap_or_else(|| "Skate 3 Rust Engine".into()),
                    resolution: (1280, 800).into(),
                    ..default()
                }),
                ..default()
            })
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(WgpuSettings {
                    backends: Some(Backends::VULKAN),
                    // Existing machine's validation layer rejects wgpu atomic shaders.
                    // This workaround belongs only to the rendering adapter.
                    instance_flags: InstanceFlags::empty(),
                    ..default()
                }),
                ..default()
            })
            .build()
            .disable::<bevy::log::LogPlugin>()
            // Gameplay and menu navigation both use raw XInput. No game system
            // consumes Bevy gamepad events/rumble; its second device backend can
            // stall PreUpdate (70.68 ms in the University capture).
            .disable::<bevy::gilrs::GilrsPlugin>();
    #[cfg(feature = "xr")]
    let plugins = if config.xr.is_some() {
        crate::xr::plugins(plugins)
    } else {
        plugins
    };
    #[cfg(feature = "xr")]
    let xr = config.xr;
    #[cfg(feature = "xr")]
    let xr_files = config.asset_root.parent().unwrap_or(&config.asset_root).to_path_buf();
    app.add_plugins(plugins)
        .insert_resource(bevy::winit::WinitSettings {
            focused_mode: bevy::winit::UpdateMode::Continuous,
            unfocused_mode: bevy::winit::UpdateMode::Continuous,
        })
        .insert_resource(crate::skate_audio::player::PlayerAudioAssets(config.asset_root.clone()))
        .insert_resource(config)
        .insert_resource(crate::retail_render::RetailScene(retail_scene))
        .insert_resource(assets::AssetManifest(manifest))
        .insert_resource(graphs)
        .insert_resource(physics)
        .insert_resource(skater);
    #[cfg(feature = "xr")]
    if let Some(options) = xr {
        // (the room's floor at the map's spawn: the test course's landing floor without a map)
        let origin = match &app.world().resource::<crate::config::Config>().map {
            Some(map) => Vec3::from_array(map.spawn),
            None => Vec3::new(0.0, crate::physics::ground::FLOOR_HEIGHT, 0.0),
        };
        app.insert_resource(crate::xr::rig::WorldOrigin(origin));
        app.add_plugins(crate::xr::XrPlugin { options, files: xr_files.clone() });
        // In the headset the faithful port is one engine of three: switched to and from while the app runs.
        // (its systems are the ones its plugins put in `SKATE3`: `add_retail`)
        crate::engines::add_engine(&mut app, Engine::Skate3, |_| Ok(()), |app| {
            add_retail(app);
            crate::xr::add_retail_link(app);
        });
        crate::clean::add_xr_engines(&mut app, &xr_files, crate::engines::Engine::Skate3);
        return app;
    }
    add_retail(&mut app);
    // SK8_ROOM_LIGHT: the room light on the desktop's test world (see `xr::room_light`).
    #[cfg(feature = "xr")]
    if xr.is_none() && let Some(source) = crate::xr::room_light::preview_source() {
        app.add_plugins(crate::xr::room_light::RoomLightPlugin(source));
    }
    app
}

/// The faithful port's plugins and systems. Each per-frame system is in [`SKATE3`]: the plugins' own sets are put
/// in it here, and their other systems join it where they are added.
fn add_retail(app: &mut App) {
    app.configure_sets(
        FixedUpdate,
        (SimulationSet::Input, SimulationSet::Controls, SimulationSet::Physics)
            .chain()
            .in_set(SKATE3),
    )
    .configure_sets(
        Update,
        (FrameSet::Assets, FrameSet::Physics, FrameSet::Animation, FrameSet::Verification)
            .chain()
            .in_set(SKATE3),
    )
    .configure_sets(
        PreUpdate,
        (crate::map_transition::MapTransitionSet, crate::graphics_menu::MenuInput).in_set(SKATE3),
    );
    hold(app);
    app.add_plugins(crate::fps_overlay::FpsOverlayPlugin)
    .add_plugins((
        crate::retail_render::RetailRenderPlugin,
        input::InputPlugin,
        PhysicsPlugin,
        crate::presentation::PresentationPlugin,
        crate::replay::ReplayPlugin,
        assets::GameAssetsPlugin,
        animation::AnimationPlugin,
        world::WorldPlugin,
        crate::grind_world::GrindGeometryPlugin,
        camera::CameraPlugin,
        crate::graphics_menu::GraphicsMenuPlugin,
        crate::map_transition::MapTransitionPlugin,
        crate::render_capacity::RenderCapacityPlugin,
        verification::VerificationPlugin,
        crate::performance::PerformancePlugin,
    ));
    app.add_plugins((crate::session_marker::SessionMarkerPlugin, crate::customiser::CustomiserPlugin));
    app.add_plugins(crate::custom_models::CustomModelsPlugin);
    app.add_plugins(crate::modding::ModdingPlugin);
    app.add_plugins(crate::skate_audio::SkateAudioPlugin);
    crate::teleport_menu::install(app);
    app.add_plugins(crate::updater::UpdaterPlugin);
    app.add_plugins(crate::multiplayer::MultiplayerPlugin);
    app.add_plugins(crate::scoring_hud::ScoringHudPlugin);
    app.add_systems(Last, crate::crash_context::sample.in_set(SKATE3));
    // (the profiler's frame markers stay shared: they time every frame, whichever engine runs)
    crate::profiling::install(app);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    #[derive(Resource, Default)]
    struct Seen {
        ticks: u32,
        alphas: Vec<f32>,
    }

    /// Skate 3 registered as the headset does it: a fixed-tick system and a drawing one in its set.
    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            // (15 ms frames on a 10 ms tick: one or two ticks a frame, and the blend between them moves)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(15)))
            .insert_resource(Time::<Fixed>::from_duration(Duration::from_millis(10)))
            .init_resource::<Seen>();
        crate::engines::add_engine(&mut app, Engine::Skate3, |_| Ok(()), |app| {
            hold(app);
            app.add_systems(FixedUpdate, (|mut seen: ResMut<Seen>| seen.ticks += 1).in_set(SKATE3))
                .add_systems(
                    Update,
                    (|hold: Res<Hold>, fixed: Res<Time<Fixed>>, mut seen: ResMut<Seen>| {
                        seen.alphas.push(hold.alpha(&fixed))
                    })
                    .in_set(SKATE3),
                );
        });
        app
    }

    fn set_paused(app: &mut App, paused: bool) {
        app.world_mut().resource_scope(|world, mut hold: Mut<Hold>| {
            hold.set(paused, world.resource::<Time<Fixed>>())
        });
    }

    fn frames(app: &mut App, n: usize) -> (u32, Vec<f32>) {
        let before = app.world().resource::<Seen>().ticks;
        app.world_mut().resource_mut::<Seen>().alphas.clear();
        for _ in 0..n {
            app.update();
        }
        let seen = app.world().resource::<Seen>();
        (seen.ticks - before, seen.alphas.clone())
    }

    /// A menu open in the headset: no fixed ticks and the drawing's blend frozen; closed again: it carries on from
    /// there, one frame's ticks at a time (no catching up on the time it was paused).
    #[test]
    fn a_held_skate3_stops_its_tick_and_resumes_without_catching_up() {
        let mut app = app();
        let (ticks, alphas) = frames(&mut app, 20);
        assert!(ticks >= 20, "it runs: {ticks} ticks in 20 frames of 15 ms");
        assert!(alphas.windows(2).any(|w| w[0] != w[1]), "the blend moves while it runs: {alphas:?}");

        set_paused(&mut app, true);
        let (ticks, alphas) = frames(&mut app, 100);
        assert_eq!(ticks, 0, "no ticks while paused");
        assert!(alphas.windows(2).all(|w| w[0] == w[1]), "the blend is frozen while paused: {alphas:?}");

        set_paused(&mut app, false);
        let (ticks, _) = frames(&mut app, 1);
        assert!(ticks <= 2, "the first frame back steps as any 15 ms frame does, not 1.5 s worth: {ticks}");
        let (ticks, _) = frames(&mut app, 20);
        assert!((20..=40).contains(&ticks), "{ticks}");
    }
}
