//! Skate 3 hosted, digested: the real game (`SKATE3_ASSET_ROOT`; skipped without it) given a floor and a rail, put
//! down, pushed, steered, flicked into an ollie and held a while, one tick a frame; each tick's link, trick text and
//! pose (root, joints, cut) written down and digested. Recorded through the app's own Skate 3 plugins and adapter
//! (`xr::link_retail`) before step 8 moves Skate 3 onto the generic driver, to check the move against.
//!
//! The script is by frame ([`Script::at`]): a tick each, but none while held. What it gives takes effect before that
//! frame's tick; each host says when to give it for that.
use bevy::prelude::*;
use std::fmt::Write;

/// The ticks run (one a frame) and the ones held.
pub(crate) const TICKS: u32 = 240;
pub(crate) const HELD: std::ops::Range<u32> = 150..175;

/// What takes effect before a frame's tick.
#[derive(Default)]
pub(crate) struct Script {
    pub stage: bool,
    pub place: Option<engine_core::PlacePlayer>,
    pub touch: engine_core::TouchState,
    pub held: engine_core::PadHeld,
}

impl Script {
    pub(crate) fn at(t: u32) -> Self {
        // (the first frame has no tick: no time has passed)
        let mut s = Script { stage: t == 1, ..default() };
        if t == 3 {
            s.place = Some(engine_core::PlacePlayer { point: Vec3::new(0.0, 0.0, -2.0), forward: Vec3::Z });
        }
        let touch = &mut s.touch;
        // (pushes, then steering, then an ollie: the right stick back and flicked forward)
        touch.x = matches!(t, 12..=16 | 32..=36 | 52..=56);
        if (70..100).contains(&t) {
            touch.left_stick = [0.35, 0.0];
        }
        match t {
            110..=115 => touch.right_stick = [0.0, -1.0],
            116..=118 => touch.right_stick = [0.0, 1.0],
            _ => {}
        }
        if HELD.contains(&t) {
            s.held = engine_core::PadHeld { held: true, pause: true };
        }
        s
    }

    /// The floor (60 m, the room's) and a rail along it.
    pub(crate) fn stage() -> (engine_core::Stage, engine_core::skate::SkatePark) {
        let mut stage = engine_core::Stage { revision: 1, ..default() };
        stage.room_quads.push([Vec3::new(-30.0, 0.0, -30.0), Vec3::new(30.0, 0.0, -30.0), Vec3::new(30.0, 0.0, 30.0), Vec3::new(-30.0, 0.0, 30.0)]);
        let park = engine_core::skate::SkatePark { revision: 1, rails: vec![vec![Vec3::new(1.5, 0.3, 0.0), Vec3::new(1.5, 0.3, 6.0)]], ..default() };
        (stage, park)
    }
}

/// A tick written down.
pub(crate) fn line(out: &mut String, tick: u32, link: &engine_core::skate::SkaterLink, trick: &engine_core::skate::TrickText, pose: &engine_core::Pose) {
    let _ = writeln!(out, "{tick}: {link:?} {trick:?} cut={} root={:?} joints={:?}", pose.discontinuity, pose.root, pose.joints);
}

pub(crate) fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// The run's digest, recorded through today's adapter (at 5f8cd2e: placed at tick 2, pushing from tick 40, an ollie
/// landed around tick 175, on the floor at its 8.7 cm deck height).
pub(crate) const RECORDED: u64 = 0xb605_958c_0fd9_cde4;

/// The real game's data root, if given.
pub(crate) fn root() -> Option<std::path::PathBuf> {
    std::env::var_os("SKATE3_ASSET_ROOT").map(Into::into)
}

/// Through the app's Skate 3 plugins (its input, physics and presentation, and the XR adapter), as the headset hosts
/// it: a tick a frame.
#[cfg(test)]
mod adapter {
    use super::*;
    use crate::xr::link::{CurrentPark, CurrentStage, PlacePlayer, SkaterLink};
    use bevy::time::TimeUpdateStrategy;

    #[derive(Resource, Default)]
    struct Written {
        out: String,
        ticks: u32,
    }

    fn write_tick(
        physics: Res<crate::physics::GamePhysics>,
        skater: Res<crate::physics::SkaterRuntime>,
        camera: Res<crate::camera::CameraRuntime>,
        mut written: ResMut<Written>,
        mut pose: Local<engine_core::Pose>,
    ) {
        let Some(frame) = camera.frame else { return };
        skate3_engine::model::pose(&skater, &frame, &mut pose);
        let link = skate3_engine::stage::link(&physics, &skater);
        let trick = skate3_engine::trick::text(&skater);
        let tick = written.ticks;
        line(&mut written.out, tick, &link, &trick, &pose);
        written.ticks += 1;
    }

    #[test]
    fn skate3_hosted_by_its_adapter() {
        let Some(root) = root() else {
            eprintln!("SKATE3_ASSET_ROOT not set: skipped");
            return;
        };
        let bundle = skate3_sim::Bundle::open(&root, &mut |_| {}).expect("the bundle");
        let config = skate3_sim::SimConfig { difficulty: skate3_sim::difficulty::Difficulty::Easy, map: None, spawn_offset: 0.0 };
        let mut sim = skate3_sim::Sim::load(&bundle, &config, &mut |_| {}).expect("the sim");
        sim.load_camera(&root).expect("its camera");
        let parts = sim.into_parts();
        let period = parts.physics.period();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(period))
            .insert_resource(crate::config::Config {
                asset_root: root.clone(),
                difficulty: crate::difficulty::Difficulty::Easy,
                check_assets: false,
                xr: None,
            })
            .insert_resource(crate::graph_runtime::StockGraphs(parts.graphs))
            .insert_resource(crate::physics::GamePhysics(parts.physics))
            .insert_resource(crate::physics::SkaterRuntime(parts.skater))
            .insert_resource(crate::camera::CameraRuntime(parts.camera.expect("its camera")))
            .insert_resource(crate::xr::rig::Rig { scale: 1.0, world_origin: Vec3::ZERO, anchor: None })
            .init_resource::<CurrentStage>()
            .init_resource::<CurrentPark>()
            .init_resource::<SkaterLink>()
            .init_resource::<engine_api::LatestTouch>()
            .init_resource::<engine_api::PadHeld>()
            .init_resource::<crate::xr::ui::hud::TrickHud>()
            .init_resource::<Written>()
            .add_message::<PlacePlayer>()
            .add_message::<crate::skate_audio::PlayerAudioObservation>()
            .configure_sets(
                FixedUpdate,
                (crate::app::SimulationSet::Input, crate::app::SimulationSet::Controls, crate::app::SimulationSet::Physics)
                    .chain()
                    .in_set(crate::app::SKATE3),
            );
        crate::app::hold_for_tests(&mut app);
        app.init_resource::<crate::replay::Replay>()
            .add_plugins((crate::input::InputPlugin, crate::physics::PhysicsPlugin, crate::presentation::PresentationPlugin, crate::xr::link::RetailLinkPlugin))
            .add_systems(FixedUpdate, write_tick.after(crate::app::SimulationSet::Physics).in_set(crate::app::SKATE3));
        let (stage, park) = Script::stage();
        // (a tick a frame, but none while held: the script is by frame)
        for frame in 0..TICKS {
            let script = Script::at(frame);
            // (the stage and placements are taken in the frame's update, after its tick: given a frame early)
            let next = Script::at(frame + 1);
            if next.stage {
                app.insert_resource(CurrentStage(stage.clone()));
                **app.world_mut().resource_mut::<CurrentPark>() = park.clone();
            }
            if let Some(place) = next.place {
                app.world_mut().write_message(PlacePlayer(place));
            }
            app.world_mut().resource_mut::<engine_api::LatestTouch>().0 = script.touch;
            **app.world_mut().resource_mut::<engine_api::PadHeld>() = script.held;
            app.world_mut().resource_mut::<crate::input::ExternalPad>().set_input(&script.touch, script.held);
            app.update();
        }
        let written = app.world().resource::<Written>();
        eprintln!("skate3 hosted: {} ticks over {TICKS} frames, digest {:016x}", written.ticks, fnv(&written.out));
        // (none the first frame: no time has passed)
        assert_eq!(written.ticks, TICKS - HELD.len() as u32 - 1, "a tick a frame, none held");
        if let Some(path) = std::env::var_os("SKATE3_GATE_DUMP") {
            std::fs::write(path, &written.out).unwrap();
        }
        assert_eq!(fnv(&written.out), RECORDED);
    }
}
