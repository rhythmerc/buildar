//! The headset menu's shared pages, the same whatever the engine: the start
//! page, the first-run intro, AR settings, building a park, controls, the
//! engine choice and the game data pages (the installer's: `super::import`).
//! An engine adds its own (SK8's skater page).
#[cfg(any(feature = "retail", feature = "thug2"))]
use super::import::{Import, Offered, Status};
use super::{Act, ControlsSheet, Outcome, PageId, UiPage, UiProvider, UiProviders, Widget, XrUi};
use crate::engines::{Engine, Engines};
use crate::xr::actions::Hand;
use crate::xr::ar_settings::{ArMode, ArSettings, HudHand};
use crate::xr::build::{BuildMode, MAX_PARK_SCALE, MAX_PIECE_SCALE, MAX_SCALE, MIN_PARK_SCALE, MIN_PIECE_SCALE, MIN_SCALE, PlacedPieces};
use crate::xr::link::{CurrentStage, PlacePlayer};
use crate::xr::planes::{RoomMesh, SceneCapture, SceneShapes};
use crate::xr::rig::Rig;
use crate::xr::room_light::RoomLight;
use crate::xr::spawn::ray_triangle;
use bevy::prelude::*;
use bevy_mod_xr::camera::XrCamera;

/// The saved parks ([`super::parks`]): what the menu opens on.
pub(crate) const PARKS: PageId = "parks";
pub(crate) const START: PageId = "start";
pub(crate) const INTRO: PageId = "intro";
const INTRO_MODE: PageId = "intro_mode";
const INTRO_FLOOR: PageId = "intro_floor";
const INTRO_CONTROLS: PageId = "intro_controls";
pub(crate) const AR: PageId = "ar";
pub(crate) const BUILD: PageId = "build";
pub(crate) const CONTROLS: PageId = "controls";
pub(crate) const ENGINE: PageId = "engine";
const BUILD_CONTROLS: PageId = "build_controls";
/// The game data pages: every engine's, and each one's.
pub(crate) const DATA: PageId = "data";
#[cfg(any(feature = "retail", feature = "thug2"))]
const DATA_PAGES: [(Engine, PageId); 3] = [(Engine::Sk8, "data_sk8"), (Engine::Skate3, "data_skate3"), (Engine::Thug2, "data_thug2")];

/// Where "put the skater in front of me" aims, real metres ahead of the head.
const AHEAD_M: f32 = 0.6;
/// Import buttons, one per copy found.
#[cfg(any(feature = "retail", feature = "thug2"))]
const IMPORT_IDS: [&str; 4] = ["import0", "import1", "import2", "import3"];

/// The running engine (none before the first frame).
fn running(world: &World) -> Option<Engine> {
    world.get_resource::<Engines>().and_then(Engines::current)
}

pub(crate) struct Shared;

impl UiProvider for Shared {
    fn page(&self, id: PageId, world: &World) -> Option<UiPage> {
        let settings = world.resource::<ArSettings>();
        let engine = running(world);
        Some(match id {
            // (drawn by `super::parks`; its title is for the trail of the pages over it)
            PARKS => UiPage::new("Parks", Vec::new()),
            START => {
                let mut w = vec![Widget::button("skate", "Skate")];
                for provider in UiProviders::active(world) {
                    w.extend(provider.start_items(world));
                }
                w.extend([
                    Widget::link(format!("Engine: {}", engine.map_or("-", Engine::name)), ENGINE),
                    Widget::link("AR settings", AR),
                    Widget::link("Build a park", BUILD),
                    Widget::button("recenter", "Place the skater: point, then A"),
                    Widget::link("Controls", CONTROLS),
                    #[cfg(any(feature = "retail", feature = "thug2"))]
                    Widget::link("Game data", DATA),
                    Widget::info(""),
                    Widget::info(tier_line(engine)),
                    Widget::info(mode_line(settings)),
                ]);
                UiPage::new("Settings", w)
            }
            INTRO => UiPage::new(
                "Welcome to SK8 AR",
                vec![
                    Widget::info("Skate a little skater around your real room: floors, tables,"),
                    Widget::info("couches, and ramps and rails you build yourself."),
                    Widget::info(tier_line(engine)),
                    Widget::info(""),
                    Widget::info("Point with the right controller and pull the trigger,"),
                    Widget::info("or move with a stick and press A. B goes back."),
                    Widget::info("Tap the left menu button any time for this menu."),
                    Widget::info(""),
                    Widget::button("next", "Start"),
                    Widget::button("skip", "Skip the intro"),
                ],
            ),
            INTRO_MODE => UiPage::new(
                "Where are you skating?",
                vec![
                    Widget::button("room", "In this room (use my room scan)"),
                    Widget::button("room_scan", "In this room: scan it now"),
                    Widget::button("free", "Outside / free roam (no scan, an endless floor)"),
                    Widget::info(""),
                    Widget::info("A room scan lets the skater ride your floor, walls, tables"),
                    Widget::info("and furniture. Free roam is one flat floor at your feet."),
                    Widget::info("Change it any time in AR settings."),
                ],
            ),
            INTRO_FLOOR => {
                let mut w = Vec::new();
                if settings.free_roam() {
                    w.push(Widget::info("Free roam: one flat floor at your feet, as far as you go."));
                    w.extend(floor_widgets(settings));
                } else {
                    w.push(Widget::info("The skater rides your floor, tables and furniture."));
                    if world.resource::<SceneCapture>().busy() {
                        w.push(Widget::info("Room scan running: finish it, then come back here."));
                    }
                }
                w.extend([
                    Widget::button("recenter", "Put the skater in front of me"),
                    Widget::link("Skater size", BUILD),
                    Widget::button("next", "Next"),
                ]);
                UiPage::new("Put the skater down", w)
            }
            INTRO_CONTROLS => {
                let mut w: Vec<Widget> = controls_rows(world).into_iter().take(9).collect();
                w.push(Widget::link("All the controls", CONTROLS));
                w.push(Widget::button("done", "Skate!"));
                UiPage::new("Controls", w)
            }
            AR => UiPage::new("AR settings", ar_widgets(world)),
            BUILD => UiPage::new("Build a park", build_widgets(world)),
            CONTROLS => {
                let mut w = controls_rows(world);
                w.push(Widget::link("Build mode controls", BUILD_CONTROLS));
                UiPage::new("Controls", w)
            }
            BUILD_CONTROLS => UiPage::new("Build mode controls", BUILD_HELP.iter().map(|(a, b)| pair(a, b)).collect()),
            ENGINE => UiPage::new("Skate engine", engine_widgets(world)),
            #[cfg(any(feature = "retail", feature = "thug2"))]
            DATA => UiPage::new("Game data", data_widgets(world)),
            #[cfg(any(feature = "retail", feature = "thug2"))]
            page if let Some(engine) = data_engine(page) => UiPage::new(format!("{} data", engine.name()), files_widgets(world, engine)),
            _ => return None,
        })
    }

    fn act(&self, page: PageId, widget: &'static str, act: Act, world: &mut World) -> Option<Outcome> {
        let page_widget = (page, widget);
        Some(match page_widget {
            (_, "skate") => Outcome::Close,
            (INTRO_FLOOR, "recenter") => {
                put_skater_ahead(world);
                Outcome::Say("The skater is in front of you".into())
            }
            (_, "recenter") => {
                world.resource_mut::<crate::xr::spawn::PlaceSkater>().0 = true;
                Outcome::Close
            }
            (INTRO, "next") => Outcome::Replace(INTRO_MODE),
            (INTRO, "skip") | (INTRO_CONTROLS, "done") => {
                edit(world, |s| s.onboarding_done = true);
                // (on to the parks; with no engine, to the game data first)
                if no_engine(world) { Outcome::Replace(DATA) } else { Outcome::Home }
            }
            (INTRO_MODE, "room" | "room_scan" | "free") => {
                let mode = if widget == "free" { ArMode::FreeRoam } else { ArMode::Room };
                edit(world, |s| s.mode = mode);
                if widget == "room_scan" {
                    world.resource_mut::<SceneCapture>().requested = true;
                }
                Outcome::Replace(INTRO_FLOOR)
            }
            (INTRO_FLOOR, "next") => Outcome::Replace(INTRO_CONTROLS),
            (_, "room_light_strength") => {
                if let Act::Set(v) = act {
                    edit(world, |s| s.room_light_strength = v);
                }
                Outcome::Stay
            }
            (_, "floor_offset") => {
                if let Act::Set(v) = act {
                    edit(world, |s| s.floor_offset = v);
                }
                Outcome::Stay
            }
            (_, "floor_here") => match right_controller(world) {
                Some(c) => {
                    let y = (c.translation.y * 100.0).round() / 100.0;
                    edit(world, |s| s.floor_offset = y.clamp(-1.0, 1.0));
                    Outcome::Say(format!("Floor set {y:+.2} m"))
                }
                None => Outcome::Say("No right controller".into()),
            },
            (ENGINE, _) => engine_act(widget, world),
            (AR, _) => ar_act(widget, world),
            (BUILD | INTRO_FLOOR, _) => build_act(widget, act, world)?,
            #[cfg(any(feature = "retail", feature = "thug2"))]
            (page, _) if let Some(engine) = data_engine(page) => files_act(engine, widget, world),
            _ => return None,
        })
    }
}

fn pair(a: &str, b: &str) -> Widget {
    Widget::info(format!("{a:<26} {b}"))
}

fn controls_rows(world: &World) -> Vec<Widget> {
    ControlsSheet::rows(world).iter().map(|c| pair(&c.input, &c.action)).collect()
}

fn tier_line(engine: Option<Engine>) -> &'static str {
    match engine {
        Some(Engine::Skate3) => "Playing your own Skate 3 data (retail controls and skaters).",
        Some(Engine::Thug2) => "Playing THUG2's own engine, on your copy of the game.",
        Some(Engine::Sk8) => "SK8: every move, value and sound in it was made for this app.",
        None => "No game yet: bring your own game files (Settings > Game data).",
    }
}

/// No engine runs, nor is one about to: the app has none that can (`crate::engines`).
pub(crate) fn no_engine(world: &World) -> bool {
    world.get_resource::<Engines>().is_some_and(|e| e.current().is_none() && e.settled().is_none())
}

// ---------------------------------------------------------------- engine

/// Each engine's button.
const ENGINE_IDS: [(&str, Engine); 3] =
    [("engine_sk8", Engine::Sk8), ("engine_skate3", Engine::Skate3), ("engine_thug2", Engine::Thug2)];

fn engine_about(engine: Engine) -> &'static str {
    match engine {
        Engine::Sk8 => "SK8: the clean-room engine, made for this app.",
        Engine::Skate3 => "Skate 3: the faithful port, on your imported Skate 3 data.",
        Engine::Thug2 => "THUG2: the game's own engine (decomp), on your THUG2 files.",
    }
}

/// The engines the menus offer: listed ([`Engines::listed`]) and either in this app or with game files to bring.
fn offered_engines(world: &World) -> Vec<Engine> {
    let Some(engines) = world.get_resource::<Engines>() else { return Vec::new() };
    Engine::ALL
        .into_iter()
        .filter(|&e| engines.listed(e, world) && (engines.carries(e) || files_of(world, e).is_some()))
        .collect()
}

fn engine_widgets(world: &World) -> Vec<Widget> {
    let Some(engines) = world.get_resource::<Engines>() else { return vec![Widget::info("One engine in this app.")] };
    let current = engines.current();
    let offered = offered_engines(world);
    let mut w = Vec::new();
    for (id, engine) in ENGINE_IDS.into_iter().filter(|(_, e)| offered.contains(e)) {
        let label = match engines.available(engine, world) {
            Ok(()) => engine.name().to_string(),
            Err(why) => format!("{}  ({})", engine.name(), files_state(world, engine).map_or(why, str::to_string)),
        };
        w.push(Widget::toggle(id, label, current == Some(engine)));
    }
    w.push(Widget::info(""));
    w.push(Widget::info("Switch any time; each engine carries on where it left off."));
    w.extend(offered.iter().map(|&e| Widget::info(engine_about(e))));
    // (developers: SK8 offered too)
    if super::sound_lab::dev(world) {
        w.push(Widget::info(""));
        w.push(Widget::toggle("sk8_shown", "Offer SK8, the clean-room engine (developers)", world.resource::<ArSettings>().sk8));
    }
    w
}

fn engine_act(widget: &str, world: &mut World) -> Outcome {
    if widget == "sk8_shown" {
        edit(world, |s| s.sk8 = !s.sk8);
        return Outcome::Stay;
    }
    let Some(&(_, engine)) = ENGINE_IDS.iter().find(|(id, _)| *id == widget) else { return Outcome::Stay };
    let Some(engines) = world.get_resource::<Engines>() else { return Outcome::Stay };
    if engines.current() == Some(engine) {
        return Outcome::Say(format!("{} is running", engine.name()));
    }
    match engines.available(engine, world) {
        Ok(()) => {
            world.resource_mut::<Engines>().request(engine);
            Outcome::Say(format!("Switched to {}", engine.name()))
        }
        Err(why) => Outcome::Say(format!("{}: {why}", engine.name())),
    }
}

/// The game files `engine` runs on, when the player brings them (the installer's: `super::import`).
fn files_of(world: &World, engine: Engine) -> Option<engine_core::GameFiles> {
    #[cfg(any(feature = "retail", feature = "thug2"))]
    {
        world.get_resource::<Offered>()?.of(engine).copied()
    }
    #[cfg(not(any(feature = "retail", feature = "thug2")))]
    {
        let _ = (world, engine);
        None
    }
}

/// Where `engine`'s game files are, when it can't run for want of them: none brought, or brought but loaded only when
/// the app next opens (an engine not carried at launch).
fn files_state(world: &World, engine: Engine) -> Option<&'static str> {
    #[cfg(any(feature = "retail", feature = "thug2"))]
    {
        let files = files_of(world, engine)?;
        let engines = world.get_resource::<Engines>()?;
        // (running on files from elsewhere: a desktop's own folder)
        if engines.available(engine, world).is_ok() {
            return None;
        }
        let installed = world.get_resource::<Import>().is_some_and(|i| i.installed(&files));
        match (installed, engines.carries(engine)) {
            (false, _) => Some("needs your game files"),
            (true, false) => Some("imported: reopen the app to load it"),
            (true, true) => None,
        }
    }
    #[cfg(not(any(feature = "retail", feature = "thug2")))]
    {
        let _ = (world, engine);
        None
    }
}

fn mode_line(settings: &ArSettings) -> &'static str {
    match settings.mode {
        ArMode::Room => "AR: your room scan.",
        ArMode::FreeRoam => "AR: free roam (one endless floor).",
    }
}

/// Change the AR settings (saved, and collision / overlays follow).
pub(crate) fn edit(world: &mut World, f: impl FnOnce(&mut ArSettings)) {
    let mut settings = world.resource_mut::<ArSettings>();
    f(&mut settings);
    settings.revision += 1;
}

fn right_controller(world: &mut World) -> Option<Transform> {
    let mut hands = world.query::<(&Hand, &Transform)>();
    hands.iter(world).find(|(h, _)| matches!(h, Hand::Right)).map(|(_, t)| *t)
}

fn floor_widgets(settings: &ArSettings) -> [Widget; 2] {
    [
        Widget::slider(
            "floor_offset",
            "Floor height",
            settings.floor_offset,
            (-1.0, 1.0, 0.01),
            format!("{:+.2} m", settings.floor_offset),
        ),
        Widget::button("floor_here", "Set it from the right controller (hold it on the ground, press A)"),
    ]
}

// ---------------------------------------------------------------- AR

fn ar_widgets(world: &World) -> Vec<Widget> {
    let s = world.resource::<ArSettings>();
    let mut w = vec![
        Widget::info(mode_line(s)),
        Widget::toggle("free_roam", "Free roam (outside: no room scan, endless floor)", s.free_roam()),
    ];
    if s.free_roam() {
        w.extend(floor_widgets(s));
    } else {
        let busy = world.resource::<SceneCapture>().busy();
        w.push(Widget::button("scan", if busy { "Room scan running..." } else { "Set up a room scan (Space Setup)" }));
    }
    w.extend([
        Widget::toggle("floor_collide", "Floor: skate on it", s.floor.collide),
        Widget::toggle("floor_show", "Floor: show it", s.floor.show),
        Widget::toggle("planes_collide", "Walls, tables, furniture: skate on them", s.planes.collide),
        Widget::toggle("planes_show", "Walls, tables, furniture: show them", s.planes.show),
        Widget::toggle("mesh_collide", "3D room mesh: skate on it", s.mesh.collide),
        Widget::toggle("mesh_show", "3D room mesh: show it", s.mesh.show),
        Widget::button("show_all", "Show everything"),
        Widget::button("hide_all", "Hide everything (still skated)"),
    ]);
    if !s.floor.collide {
        w.push(Widget::info("Floor off: the skater falls through it where nothing else is."));
    }
    let shapes = world.resource::<SceneShapes>().0.len();
    let triangles = world.resource::<RoomMesh>().0.as_ref().map_or(0, |m| m.indices.len() / 3);
    w.push(Widget::toggle("room_light", "Roomtone room lighting: light the skater and the park by your room", s.room_light));
    if s.room_light {
        w.push(Widget::slider(
            "room_light_strength",
            "Roomtone brightness",
            s.room_light_strength,
            (0.25, 2.0, 0.05),
            format!("{:.0}%", s.room_light_strength * 100.0),
        ));
    }
    if let Some(problem) = world.resource::<RoomLight>().problem(s.room_light) {
        w.push(Widget::info(problem));
    }
    // (developers: Roomtone's session, recorded from the start when asked for, saved to replay at a desk)
    if s.room_light && super::sound_lab::dev(world) {
        if let Some(session) = world.get_resource::<crate::xr::room_light::session::RoomSession>() {
            w.push(Widget::toggle("room_record", "Record Roomtone for captures (applies on restart)", s.room_record));
            if session.recording() {
                w.push(Widget::button("room_light_save", "Save the Roomtone session"));
            }
            let status = session.status();
            if !status.is_empty() {
                w.push(Widget::info(status));
            }
        }
    }
    w.push(Widget::toggle("contact_shadows", "Contact shadows (board and park pieces)", s.contact_shadows));
    w.push(Widget::toggle("occlusion", "Real things hide the skater and park", s.occlusion.on()));
    if s.occlusion.on() {
        let problem = *world.resource::<crate::xr::occlusion::OcclusionProblem>().0.lock().unwrap();
        w.push(Widget::info(problem.unwrap_or("Uses the headset's depth sensing.")));
        w.push(Widget::toggle("occlusion_edges", "Sharper edges, found by the headset's cameras", s.occlusion_edges));
        w.push(Widget::toggle("occlusion_steady", "Steadier edges, remembered from frame to frame", s.occlusion_steady));
        w.push(Widget::toggle("occlusion_hands", "Hands keep up, moved with the controllers", s.occlusion_hands));
        w.push(Widget::toggle("occlusion_controllers", "Controllers hide things too, by their own shapes", s.occlusion_controllers));
        w.push(Widget::toggle(
            "hud_depth_at_controller",
            "HUD hidden by real things in front of the controller (off: never hidden)",
            s.hud_depth_at_controller,
        ));
        if s.occlusion_edges && !crate::camera_access() {
            w.push(Widget::info("The cameras aren't allowed yet: turn this off and on to be asked again."));
        }
        // (developers: what occlusion works from, saved to work on at a desk)
        if super::sound_lab::dev(world) {
            w.push(Widget::button("occlusion_capture", format!("Record an occlusion capture ({} s)", crate::xr::occlusion_capture::SECONDS)));
            let status = world.resource::<crate::xr::occlusion_capture::Capture>().status();
            if !status.is_empty() {
                w.push(Widget::info(status));
            }
        }
    }
    w.extend([
        Widget::toggle("hud", "Trick and points readout", s.hud),
    ]);
    if s.hud {
        w.extend([
            Widget::toggle("hud_left", "Skate 3 and THUG2 HUDs by the left controller", s.hud_hand == HudHand::Left),
            Widget::toggle("hud_right", "Skate 3 and THUG2 HUDs by the right controller", s.hud_hand == HudHand::Right),
            Widget::info("THUG2's balance meter is always by the left controller."),
        ]);
    }
    w.extend([
        Widget::button("intro", "Replay the intro"),
        Widget::info(format!("Room: {shapes} surfaces, {triangles} mesh triangles")),
    ]);
    w
}

fn ar_act(widget: &str, world: &mut World) -> Outcome {
    match widget {
        "scan" => {
            world.resource_mut::<SceneCapture>().requested = true;
            return Outcome::Say("Starting Space Setup".into());
        }
        "occlusion_capture" => {
            let folder = world.resource::<crate::xr::XrFiles>().0.join("captures").join(
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()).to_string(),
            );
            let root = world.resource::<crate::xr::rig::Rig>().root_transform();
            let start = serde_json::json!({
                "kind": "start",
                "world_from_tracking": root.map(|r| r.to_matrix().to_cols_array()),
                // (the room's surfaces, in the game's world)
                "room_quads": world.resource::<CurrentStage>().room_quads.iter().map(|q| q.map(|p| p.to_array())).collect::<Vec<_>>(),
            });
            let said = match world.resource::<crate::xr::occlusion_capture::Capture>().start(folder, start) {
                Ok(()) => "Recording: look at edges that go wrong".to_string(),
                Err(why) => why,
            };
            return Outcome::Say(said);
        }
        "room_light_save" => {
            let folder = world.resource::<crate::xr::XrFiles>().0.join("captures").join(format!(
                "roomtone-{}",
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
            ));
            let note = crate::xr::room_light::session::note(world.resource::<ArSettings>());
            let (Some(session), Some(capture)) = (
                world.get_resource::<crate::xr::room_light::session::RoomSession>(),
                world.get_resource::<crate::xr::room_light::RoomCapture>(),
            ) else {
                return Outcome::Say("Roomtone isn't running".into());
            };
            // (waits out a frame being added on the capture's thread: a few ms at most)
            let said = match session.save(&mut capture.0.lock().unwrap(), folder, note) {
                Ok(_) => "Saving the Roomtone session".to_string(),
                Err(why) => why,
            };
            return Outcome::Say(said);
        }
        "intro" => {
            edit(world, |s| s.onboarding_done = false);
            world.resource_mut::<XrUi>().request = Some(INTRO);
            return Outcome::Stay;
        }
        _ => {}
    }
    edit(world, |s| match widget {
        "free_roam" => s.mode = if s.free_roam() { ArMode::Room } else { ArMode::FreeRoam },
        "floor_collide" => s.floor.collide = !s.floor.collide,
        "floor_show" => s.floor.show = !s.floor.show,
        "planes_collide" => s.planes.collide = !s.planes.collide,
        "planes_show" => s.planes.show = !s.planes.show,
        "mesh_collide" => s.mesh.collide = !s.mesh.collide,
        "mesh_show" => s.mesh.show = !s.mesh.show,
        "show_all" => s.set_all_shown(true),
        "hide_all" => s.set_all_shown(false),
        "hud" => s.hud = !s.hud,
        "hud_left" => s.hud_hand = HudHand::Left,
        "hud_right" => s.hud_hand = HudHand::Right,
        "room_light" => s.room_light = !s.room_light,
        "room_record" => s.room_record = !s.room_record,
        "contact_shadows" => s.contact_shadows = !s.contact_shadows,
        "occlusion" => s.occlusion = s.occlusion.toggled(),
        "occlusion_edges" => s.occlusion_edges = !s.occlusion_edges,
        "occlusion_steady" => s.occlusion_steady = !s.occlusion_steady,
        "occlusion_hands" => s.occlusion_hands = !s.occlusion_hands,
        "occlusion_controllers" => s.occlusion_controllers = !s.occlusion_controllers,
        "hud_depth_at_controller" => s.hud_depth_at_controller = !s.hud_depth_at_controller,
        _ => {}
    });
    if widget == "occlusion_edges" && world.resource::<ArSettings>().occlusion_edges && !crate::camera_access() {
        crate::request_camera_access();
        return Outcome::Say("Edges are found by looking at your room through the headset's cameras. Nothing is saved or sent.".into());
    }
    // The room's light is read from a passthrough camera: ask for it once.
    if widget == "room_light" && world.resource::<ArSettings>().room_light && !crate::camera_access() {
        crate::request_camera_access();
        return Outcome::Say("Roomtone looks at your room through the headset's cameras. Nothing is saved or sent.".into());
    }
    Outcome::Stay
}

// ---------------------------------------------------------------- build

/// The skater's size against the real room as a 0..1 slider: 1 is life
/// size, 0 the smallest (the world scale is game metres per real metre).
pub(crate) fn skater_size_widget(world: &World) -> Widget {
    let scale = world.resource::<Rig>().scale;
    let v = 1.0 - scale.ln() / MAX_SCALE.ln();
    let text = if scale <= 1.01 { "life size".to_string() } else { format!("1/{scale:.0} life size") };
    Widget::slider("skater_size", "Skater size", v, (0.0, 1.0, 0.01), text)
}

/// Set the skater's size from its slider. Only the skater changes: the world
/// scale moves and the park scale moves with it, so every placed piece keeps
/// its real size (and its own scale). The skater stays on the same real spot.
pub(crate) fn set_skater_size(world: &mut World, v: f32) {
    let old = world.resource::<Rig>().scale;
    let scale = MAX_SCALE.powf(1.0 - v).clamp(MIN_SCALE, MAX_SCALE);
    {
        let mut placed = world.resource_mut::<PlacedPieces>();
        placed.park_scale = (placed.park_scale * scale / old).clamp(MIN_PARK_SCALE, MAX_PARK_SCALE);
    }
    set_world_scale(world, scale);
}

/// The park-size slider is logarithmic over the wide park range.
fn park_slider(world: &World) -> Widget {
    let park = world.resource::<PlacedPieces>().park_scale;
    let span = (MAX_PARK_SCALE / MIN_PARK_SCALE).ln();
    let v = (park / MIN_PARK_SCALE).ln() / span;
    Widget::slider("park_scale", "Park size (every piece)", v, (0.0, 1.0, 0.005), format!("{park:.2}x"))
}

fn park_from_slider(v: f32) -> f32 {
    (MIN_PARK_SCALE * ((MAX_PARK_SCALE / MIN_PARK_SCALE).ln() * v).exp()).clamp(MIN_PARK_SCALE, MAX_PARK_SCALE)
}

/// The world's scale (the player is put back on the same real spot once it settles: `xr::link`).
pub(crate) fn set_world_scale(world: &mut World, scale: f32) {
    world.resource_mut::<Rig>().scale = scale;
    world.resource_mut::<PlacedPieces>().revision += 1; // saves the layout (it carries the scale)
}

fn build_widgets(world: &World) -> Vec<Widget> {
    let placed = world.resource::<PlacedPieces>();
    let mut w = vec![
        Widget::button("build", "Start building (hold the menu button 1 s to stop)"),
        skater_size_widget(world),
        park_slider(world),
    ];
    match placed.selected.and_then(|id| placed.items.iter().find(|p| p.id == id)) {
        Some(item) => w.extend([
            Widget::info(format!("Selected: {} (the last piece pointed at)", item.name())),
            Widget::slider(
                "piece_scale",
                "This piece's size",
                item.scale,
                (MIN_PIECE_SCALE, MAX_PIECE_SCALE, 0.05),
                format!("{:.2}x", item.scale),
            ),
            Widget::toggle("piece_hidden", "This piece invisible (still skated)", item.hidden),
            Widget::button("piece_delete", "Delete this piece"),
        ]),
        None => w.push(Widget::info("Select a piece in build mode (point at it, right grip) to edit it here.")),
    }
    w.extend([
        Widget::toggle("show_hidden", "Show invisible pieces while skating", placed.show_hidden),
        Widget::toggle("build_hints", "Build hints: the controls beside the controllers", world.resource::<ArSettings>().build_hints),
        Widget::toggle("build_surfaces", "Show the room's surfaces while building", world.resource::<ArSettings>().build_surfaces),
        Widget::button("undo", "Undo"),
        Widget::button("clear", "Clear every piece"),
        Widget::info(format!("{} pieces", placed.items.len())),
        Widget::link("Build mode controls", BUILD_CONTROLS),
    ]);
    w
}

fn build_act(widget: &str, act: Act, world: &mut World) -> Option<Outcome> {
    let value = match act {
        Act::Set(v) => Some(v),
        _ => None,
    };
    Some(match widget {
        "build" => {
            world.resource_mut::<BuildMode>().active = true;
            Outcome::Close
        }
        "skater_size" => {
            set_skater_size(world, value?);
            Outcome::Stay
        }
        "park_scale" => {
            let mut p = world.resource_mut::<PlacedPieces>();
            p.park_scale = park_from_slider(value?);
            p.revision += 1;
            Outcome::Stay
        }
        "piece_scale" | "piece_hidden" | "piece_delete" => {
            let mut p = world.resource_mut::<PlacedPieces>();
            let id = p.selected?;
            if widget == "piece_delete" {
                p.remove(id);
                p.selected = None;
                return Some(Outcome::Say("Piece deleted".into()));
            }
            p.checkpoint();
            let item = p.get_mut(id)?;
            if widget == "piece_scale" {
                item.scale = crate::xr::build::snap_scale(value?);
            } else {
                item.hidden = !item.hidden;
            }
            p.revision += 1;
            Outcome::Stay
        }
        "show_hidden" => {
            let mut p = world.resource_mut::<PlacedPieces>();
            p.show_hidden = !p.show_hidden;
            Outcome::Stay
        }
        "build_hints" => {
            edit(world, |s| s.build_hints = !s.build_hints);
            Outcome::Stay
        }
        "build_surfaces" => {
            edit(world, |s| s.build_surfaces = !s.build_surfaces);
            Outcome::Stay
        }
        "undo" => {
            let undone = world.resource_mut::<PlacedPieces>().undo();
            Outcome::Say(if undone { "Undone".into() } else { "Nothing to undo".into() })
        }
        "clear" => {
            world.resource_mut::<PlacedPieces>().clear();
            Outcome::Say("Cleared (Undo brings them back)".into())
        }
        _ => return None,
    })
}

const BUILD_HELP: [(&str, &str); 16] = [
    ("Left menu (hold 1 s)", "enter / leave build mode"),
    ("The legend", "beside each controller: what it does right now"),
    ("Left stick left/right", "choose the next piece"),
    ("Left stick up/down", "what it's made of: wood, concrete, brick..."),
    ("Right stick", "turn it / make it bigger or smaller"),
    ("A or trigger", "put it where you point (pieces stack)"),
    ("Right stick click", "hold it in your hand instead (free placement)"),
    ("Right grip on a piece", "select it: the sticks then change that piece"),
    ("Selected: hold right grip", "move it"),
    ("Selected: B / right stick click", "delete it / copy it"),
    ("Selected: A or trigger", "done"),
    ("Hold left grip", "second set: length, width, colour, hide"),
    ("Left trigger", "undo"),
    ("Left stick click", "drop a starter park"),
    ("Y / hold X", "rescan the room / clear every piece"),
    ("Pieces line up", "with table edges and with each other"),
];

// ---------------------------------------------------------------- data

/// The engine whose game data page `page` is.
#[cfg(any(feature = "retail", feature = "thug2"))]
fn data_engine(page: PageId) -> Option<Engine> {
    DATA_PAGES.iter().find(|(_, p)| *p == page).map(|(e, _)| *e)
}

#[cfg(any(feature = "retail", feature = "thug2"))]
fn data_page(engine: Engine) -> PageId {
    DATA_PAGES.iter().find(|(e, _)| *e == engine).map_or(DATA, |(_, p)| p)
}

/// Every engine's game files: each one's state, a link to its page.
#[cfg(any(feature = "retail", feature = "thug2"))]
fn data_widgets(world: &World) -> Vec<Widget> {
    let mut w = vec![Widget::info(tier_line(running(world))), Widget::info("")];
    let offered = offered_engines(world);
    for (e, _) in world.resource::<Offered>().0.iter().filter(|(e, _)| offered.contains(e)) {
        let state = files_state(world, *e).unwrap_or("ready");
        w.push(Widget::link(format!("{}: {state}", e.name()), data_page(*e)));
    }
    w.extend([
        Widget::info(""),
        Widget::info("The games run on your own copy of their files: nothing of"),
        Widget::info("theirs comes with this app. Bring each one's once."),
    ]);
    w
}

/// One engine's game files: where they are, importing them, and how to make them.
#[cfg(any(feature = "retail", feature = "thug2"))]
fn files_widgets(world: &World, engine: Engine) -> Vec<Widget> {
    let Some(files) = files_of(world, engine) else { return Vec::new() };
    let engines = world.resource::<Engines>();
    let import = world.resource::<Import>();
    let installed = import.installed(&files);
    let mut w = vec![Widget::info(match files_state(world, engine) {
        None => "Your files are in: it's ready to play.",
        Some(_) if !installed => "Your files aren't here yet.",
        Some(state) => state,
    })];
    if engines.available(engine, world).is_ok() && engines.current() != Some(engine) {
        w.push(Widget::button("play", format!("Play {} now", engine.name())));
    }
    if installed && !engines.carries(engine) {
        w.push(Widget::button("restart", "Close the app now (then open it again)"));
    }
    // (files an engine has started on stay as they are until the app closes)
    let in_use = engines.started(engine);
    let drop = files.drop;
    match import.status(engine) {
        Status::Working { what, done, total } => {
            let pct = if total > 0 { done as f32 / total as f32 * 100.0 } else { 0.0 };
            w.push(Widget::info(format!("Importing: {pct:.0}%  {what}")));
        }
        Status::Done(Ok(msg)) => w.push(Widget::info(msg)),
        Status::Done(Err(e)) => w.push(Widget::info(format!("Import failed: {e}"))),
        Status::Scanned(found) if found.is_empty() => {
            w.push(Widget::info(format!("Nothing found in Download/{drop}.")));
            w.push(Widget::button("scan", "Look again"));
        }
        Status::Scanned(_) if in_use => w.push(Widget::info("In use: close and reopen the app to replace them.")),
        Status::Scanned(found) => {
            for (id, source) in IMPORT_IDS.iter().zip(found.iter()) {
                w.push(Widget::button(id, format!("Import {}", source.describe())));
            }
            w.push(Widget::button("scan", "Look again"));
        }
        Status::Idle => w.push(Widget::button("scan", format!("Import from Download/{drop}"))),
    }
    w.push(Widget::info(""));
    w.extend(files.how.iter().map(|line| Widget::info(*line)));
    w.dedup();
    w
}

#[cfg(any(feature = "retail", feature = "thug2"))]
fn files_act(engine: Engine, widget: &str, world: &mut World) -> Outcome {
    let Some(files) = files_of(world, engine) else { return Outcome::Stay };
    let import = world.resource::<Import>().clone();
    match widget {
        "play" => engine_act(ENGINE_IDS.iter().find(|(_, e)| *e == engine).map_or("", |(id, _)| id), world),
        "scan" => {
            if !crate::storage_access() {
                crate::request_storage_access();
                return Outcome::Say("Allow \"All files access\" for SK8 AR, then press it again.".into());
            }
            import.scan(engine, &files);
            Outcome::Stay
        }
        "restart" => {
            info!("UI closing the app to load the imported files");
            std::process::exit(0);
        }
        id => {
            let Some(i) = IMPORT_IDS.iter().position(|&x| x == id) else { return Outcome::Stay };
            if world.resource::<Engines>().started(engine) {
                return Outcome::Say("In use: close and reopen the app to replace them".into());
            }
            if import.working() {
                return Outcome::Say("Another import is running".into());
            }
            if let Status::Scanned(found) = import.status(engine)
                && let Some(source) = found.get(i)
            {
                import.start(engine, files, source.clone());
            }
            Outcome::Stay
        }
    }
}

// ---------------------------------------------------------------- skater

/// Put the skater on the ground just ahead of the player (the nearest
/// ground-like face below that spot, else the floor height).
pub(crate) fn put_skater_ahead(world: &mut World) {
    let Some(root) = world.resource::<Rig>().root_transform() else { return };
    let mut eyes = world.query_filtered::<&Transform, With<XrCamera>>();
    let Some(eye) = eyes.iter(world).next().copied() else { return };
    let forward = (eye.rotation * Vec3::NEG_Z).with_y(0.0).normalize_or(Vec3::NEG_Z);
    let ahead_tracking = eye.translation + forward * AHEAD_M;
    let ahead = root.transform_point(ahead_tracking);
    let from = ahead + Vec3::Y * root.scale.y * 0.5;
    let ground = world
        .resource::<CurrentStage>()
        .triangles()
        .filter(|[a, b, c]| (*b - *a).cross(*c - *a).normalize_or_zero().y.abs() > 0.6)
        .filter_map(|[a, b, c]| ray_triangle(from, Vec3::NEG_Y, a, b, c))
        .min_by(f32::total_cmp)
        .map(|t| from - Vec3::Y * t);
    let floor_y = world.resource::<ArSettings>().floor_offset;
    let point = ground.unwrap_or_else(|| root.transform_point(ahead_tracking.with_y(floor_y)));
    world.write_message(PlacePlayer::new(point, root.rotation * forward));
}

#[cfg(all(test, feature = "retail", feature = "thug2"))]
mod tests {
    use super::*;
    use crate::engines::add_engine;

    fn needs_files(_: &World) -> Result<(), String> {
        Err("needs your game files".into())
    }

    fn text(world: &World, page: PageId) -> String {
        format!("{:?}", Shared.page(page, world).expect("the page").widgets)
    }

    /// No game files and SK8 not offered: the app runs no engine, and its menu offers each game's files (SK8 nowhere);
    /// SK8 turned on, it is offered again.
    #[test]
    fn with_no_game_files_the_menu_offers_them() {
        let files = std::env::temp_dir().join(format!("pages-test-{}", std::process::id()));
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).init_resource::<ArSettings>().init_resource::<SceneCapture>().init_resource::<ControlsSheet>().init_resource::<UiProviders>();
        add_engine(&mut app, Engine::Sk8, |w| if w.resource::<ArSettings>().sk8 { Ok(()) } else { Err("not offered".into()) }, |_| {});
        add_engine(&mut app, Engine::Thug2, needs_files, |_| {});
        app.world_mut().resource_mut::<Engines>().shown_if(Engine::Sk8, |w| w.resource::<ArSettings>().sk8);
        app.insert_resource(Import::new(files.clone()));
        crate::xr::ui::import::offer(&mut app, Engine::Skate3, skate3_engine::DATA);
        crate::xr::ui::import::offer(&mut app, Engine::Thug2, thug2_engine::DATA);
        app.update();
        let world = app.world();
        assert!(no_engine(world));
        let data = text(world, DATA);
        assert!(data.contains("Skate 3: needs your game files") && data.contains("THUG2: needs your game files"), "{data}");
        let thug2 = text(world, "data_thug2");
        assert!(thug2.contains("Import from Download/THUG2") && thug2.contains("docs/THUG2.md"), "{thug2}");
        let engine = text(world, ENGINE);
        assert!(!engine.contains("SK8") && engine.contains("Skate 3  (needs your game files)"), "{engine}");
        // (the pages a first run goes through, with no engine)
        for page in [START, INTRO, INTRO_MODE, INTRO_FLOOR, INTRO_CONTROLS, CONTROLS] {
            assert!(!text(world, page).contains("SK8:"), "{page}");
        }
        assert!(text(world, CONTROLS).contains("this menu"), "the shared layer's controls");
        assert!(text(world, INTRO).contains("No game yet"));

        app.world_mut().resource_mut::<ArSettings>().sk8 = true;
        let engine = text(app.world(), ENGINE);
        assert!(engine.contains("engine_sk8"), "{engine}");
        app.world_mut().resource_mut::<Engines>().request(Engine::Sk8);
        app.update();
        assert!(!no_engine(app.world()));
        let _ = std::fs::remove_dir_all(files);
    }
}
