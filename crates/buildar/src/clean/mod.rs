//! The public tier: the clean-room engine (`skate-lite-sys`, written from
//! scratch, no retail data), a skater built in code ([`skater`]), CC0
//! sounds, and the shared XR layer (room collision, build mode, spawn).
//!
//! Nothing here reads a file from an imported disc. The imported-disc tier
//! (the faithful port) runs instead when `files/use-retail-data` exists on
//! the Quest, or without `--clean` on the desktop.
mod board;
mod collide;
mod input;
mod menu;
mod mirror;
pub(crate) mod sim;
mod procedural;
mod skater;
#[cfg(feature = "xr")]
mod skater_page;
mod view;
#[cfg(feature = "xr")]
mod xr_menu;

use bevy::prelude::*;
use sim::CleanSkate;
use skate_lite_sys::{EventKind, Mode};

/// Set while a menu owns the controllers (the skater gets neutral input).
#[derive(Resource, Default)]
pub(crate) struct UiCapture(pub bool);

#[derive(Clone, Debug, Default)]
pub(crate) struct Options {
    pub xr: Option<crate::config::XrOptions>,
    pub check: bool,
    pub files: std::path::PathBuf,
}

fn options() -> Result<Options, String> {
    let mut o = Options::default();
    #[cfg(target_os = "android")]
    {
        o.xr = Some(crate::config::XrOptions::default());
        o.files = crate::android::files_dir()?;
    }
    #[cfg(not(target_os = "android"))]
    {
        o.files = std::env::current_dir().map_err(|e| e.to_string())?;
    }
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--clean" => {}
            "--check-assets" => o.check = true,
            "--xr" => {
                if !cfg!(feature = "xr") {
                    return Err("--xr needs a build with the `xr` feature".into());
                }
                o.xr.get_or_insert_default();
            }
            "--xr-scale" => {
                let s: f32 = args
                    .next()
                    .ok_or("--xr-scale requires a number")?
                    .parse()
                    .map_err(|_| "--xr-scale requires a number")?;
                o.xr.get_or_insert_default().scale = s;
            }
            "--files" => o.files = args.next().ok_or("--files requires a directory")?.into(),
            other => warn!("clean tier ignores {other}"),
        }
    }
    Ok(o)
}

/// The plugins as the headset's dev flags want them made (each flag in `xr::dev_flags`).
#[cfg(feature = "xr")]
fn dev_plugins(mut plugins: bevy::app::PluginGroupBuilder, flags: &crate::xr::dev_flags::DevFlags) -> bevy::app::PluginGroupBuilder {
    // The compute pool's threads: Bevy sizes it from the cores the app may use (on the Quest, 3: one thread). `0`
    // is none at all: then the threads running the schedules run their systems themselves.
    if let Some(threads) = flags.number("compute") {
        plugins = plugins.set(bevy::app::TaskPoolPlugin {
            task_pool_options: bevy::app::TaskPoolOptions {
                compute: bevy::app::TaskPoolThreadAssignmentPolicy {
                    min_threads: threads,
                    max_threads: if threads == 0 { 0 } else { usize::MAX },
                    percent: 1.0,
                    on_thread_spawn: None,
                    on_thread_destroy: None,
                },
                ..default()
            },
        });
    }
    let (cpu_preprocess, lean) = (flags.has("cpu_preprocess"), flags.has("lean_plugins"));
    if cpu_preprocess || lean {
        plugins = plugins.set(bevy::pbr::PbrPlugin {
            use_gpu_instance_buffer_builder: !cpu_preprocess,
            add_default_deferred_lighting_plugin: !lean,
            ..default()
        });
    }
    if lean {
        plugins = plugins.disable::<bevy::post_process::PostProcessPlugin>().disable::<bevy::anti_alias::AntiAliasPlugin>();
    }
    // The ECS's and the render graph's per-system tracing spans are filtered out of the log in the headset (Bevy's
    // `trace` feature makes one each system run, each through the Android log layer: ~0.4 ms a frame on the Quest),
    // unless the dev flag `spans` keeps them (for a profiler that reads them).
    if !flags.has("spans") {
        plugins = plugins.set(bevy::log::LogPlugin {
            filter: format!("{},bevy_ecs=warn,bevy_app=warn,bevy_render::renderer=warn,bevy_render::render_graph=warn", bevy::log::DEFAULT_FILTER),
            ..default()
        });
    }
    plugins
}

pub(crate) fn run() -> AppExit {
    let options = match options() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return AppExit::error();
        }
    };
    if options.check {
        // Everything the clean tier needs is compiled in.
        let params = skate_lite_sys::params().len();
        let tricks = skate_lite_sys::trick_names().len();
        eprintln!("SKATE_CLEAN_TIER engine params={params} tricks={tricks}");
        eprintln!("SKATE_ASSETS_READY");
        return AppExit::Success;
    }
    eprintln!("SKATE_CLEAN_TIER starting (xr={})", options.xr.is_some());
    build(options).run()
}

fn build(options: Options) -> App {
    use bevy::render::{
        RenderPlugin,
        settings::{Backends, InstanceFlags, RenderCreation, WgpuSettings},
    };
    let mut app = App::new();
    // Skate 3, carried in the headset (and `--xr`) whether its data is there yet or not: where its data is (or goes),
    // an asset source, which has to be there before the plugins.
    #[cfg(all(feature = "xr", feature = "retail"))]
    let skate3 = options.xr.map(|_| crate::skate3::assets(&options.files));
    #[cfg(not(all(feature = "xr", feature = "retail")))]
    let skate3: Option<std::path::PathBuf> = None;
    #[cfg(all(feature = "xr", feature = "retail"))]
    if let Some(assets) = &skate3 {
        crate::skate3::register_assets(&mut app, assets);
    }
    let plugins = DefaultPlugins
        .set(WindowPlugin {
            primary_window: Some(Window {
                title: "buildAR".into(),
                resolution: (1280, 800).into(),
                ..default()
            }),
            ..default()
        })
        .set(RenderPlugin {
            render_creation: RenderCreation::Automatic(WgpuSettings {
                // Metal on Apple machines (no Vulkan driver there), Vulkan elsewhere.
                backends: Some(if cfg!(target_vendor = "apple") { Backends::METAL } else { Backends::VULKAN }),
                instance_flags: InstanceFlags::empty(),
                ..default()
            }),
            ..default()
        })
        .build();
    // The headset's dev flags that change how the plugins are made (`xr::dev_flags`).
    #[cfg(feature = "xr")]
    let plugins = match options.xr.as_ref().map(|_| crate::xr::dev_flags::DevFlags::read(&options.files)) {
        Some(flags) => dev_plugins(plugins, &flags),
        None => plugins,
    };
    #[cfg(target_os = "android")]
    let plugins = plugins.disable::<bevy::gilrs::GilrsPlugin>();
    #[cfg(feature = "xr")]
    let plugins = if options.xr.is_some() { crate::xr::plugins(plugins) } else { plugins };
    app.add_plugins(plugins).insert_resource(bevy::winit::WinitSettings {
        focused_mode: bevy::winit::UpdateMode::Continuous,
        unfocused_mode: bevy::winit::UpdateMode::Continuous,
    });
    crate::fonts::embed(&mut app);

    let xr = options.xr.is_some();
    #[cfg(feature = "xr")]
    if let Some(xr_options) = options.xr {
        app.add_plugins(crate::xr::XrPlugin { options: xr_options, files: options.files.clone() });
        add_xr_engines(&mut app, &options.files, skate3);
    }
    #[cfg(not(feature = "xr"))]
    let _ = skate3;
    // SK8_ROOM_LIGHT: the room light on the desktop park (see `xr::room_light`).
    #[cfg(feature = "xr")]
    if !xr && let Some(source) = crate::xr::room_light::preview_source() {
        app.add_plugins(crate::xr::room_light::RoomLightPlugin(source));
    }
    if !xr {
        add_sk8(&mut app, &options.files, false);
    }
    // SK8_FIXED_DT=1: every frame advances exactly 1/60 s (deterministic SK8_ANIM_TRACE runs).
    if std::env::var_os("SK8_FIXED_DT").is_some() {
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f64(
            1.0 / 60.0,
        )));
        // SK8_FRAMES=n: quit after n frames, so runs cover exactly the same simulated time.
        if let Some(limit) = std::env::var("SK8_FRAMES").ok().and_then(|v| v.parse::<u32>().ok()) {
            app.add_systems(Last, move |mut n: Local<u32>, mut exit: MessageWriter<AppExit>| {
                *n += 1;
                if *n >= limit {
                    exit.write(AppExit::Success);
                }
            });
        }
    }
    // SK8_CAPTURE=<dir>: every frame advances exactly 1/60 s and is saved (f00000.jpg, ...), so a
    // capture is deterministic however slowly frames are written; SK8_CAPTURE_FRAMES stops it.
    if let Ok(dir) = std::env::var("SK8_CAPTURE") {
        let frames: u32 = std::env::var("SK8_CAPTURE_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(1620);
        let _ = std::fs::create_dir_all(&dir);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f64(
            1.0 / 60.0,
        )))
        .add_systems(Last, move |mut commands: Commands, mut n: Local<u32>, mut exit: MessageWriter<AppExit>| {
            use bevy::render::view::window::screenshot::{Screenshot, save_to_disk};
            // Skip the first frames while the skater loads.
            *n += 1;
            if *n <= 30 {
                return;
            }
            let i = *n - 31;
            if i >= frames {
                exit.write(AppExit::Success);
                return;
            }
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(format!("{dir}/f{i:05}.jpg")));
        });
    }
    app
}

/// The headset app's engines: Skate 3 when the app is built with it (`skate3`: where its data is or goes; it runs once
/// the data is there), SK8 (offered only to developers, [`sk8_shown`]), and THUG2 when the app is built with it. The
/// engine to start on is the one last played or just imported (`engine.txt` in `files`), else Skate 3, else the first
/// that can run, else none: the menu then offers the game files (`xr::ui::pages`).
#[cfg(feature = "xr")]
pub(crate) fn add_xr_engines(app: &mut App, files: &std::path::Path, skate3: Option<std::path::PathBuf>) {
    use crate::engines::{Engine, Engines};
    let sk8_files = files.to_path_buf();
    // (the skating sounds are shared, not an engine's own: installed first, outside the engines)
    crate::skate_audio::clean::install(app);
    let first = if skate3.is_some() { Engine::Skate3 } else { Engine::Sk8 };
    #[cfg(feature = "retail")]
    {
        crate::xr::ui::import::offer(app, Engine::Skate3, skate3_engine::DATA);
        if let Some(assets) = skate3 {
            crate::skate3::install(app, files, assets);
        }
    }
    #[cfg(not(feature = "retail"))]
    let _ = skate3;
    crate::engines::add_engine(app, Engine::Sk8, sk8_available, move |app| add_sk8(app, &sk8_files, true));
    #[cfg(feature = "thug2")]
    crate::thug2::install(app, files);
    #[cfg(feature = "mkwii")]
    crate::mkwii::install(app, files);
    let mut engines = app.world_mut().resource_mut::<Engines>();
    engines.shown_if(Engine::Sk8, sk8_shown);
    engines.remember_in(files.to_path_buf(), first);
}

/// SK8 is offered to developers who turn it on (AR settings), not to players: too rough yet. A build with no other
/// engine offers it anyway.
#[cfg(feature = "xr")]
fn sk8_shown(world: &World) -> bool {
    !cfg!(any(feature = "retail", feature = "thug2"))
        || world.get_resource::<crate::xr::ar_settings::ArSettings>().is_some_and(|s| s.sk8)
}

#[cfg(feature = "xr")]
fn sk8_available(world: &World) -> Result<(), String> {
    if sk8_shown(world) { Ok(()) } else { Err("not offered yet".into()) }
}

/// SK8's per-frame systems (its own when the app carries engines; on the desktop just a set).
const SK8: crate::engines::EngineSet = crate::engines::EngineSet(crate::engines::Engine::Sk8);

/// The SK8 engine: the clean-room skater and its menus, sounds and (in the headset) its adapter to the XR layer.
fn add_sk8(app: &mut App, files: &std::path::Path, xr: bool) {
    app.insert_resource(CleanSkate::new())
        .insert_resource(menu::Menu::new(files.to_path_buf(), xr))
        .init_resource::<menu::MenuNav>()
        .init_resource::<menu::HudLines>()
        .init_resource::<UiCapture>()
        .add_systems(Startup, (view::lights, skater::spawn))
        .add_systems(PostStartup, menu::spawn_ui);
    crate::skate_audio::clean::install(app);
    bevy::asset::embedded_asset!(app, "assets/skater.glb");
    #[cfg(feature = "xr")]
    if xr {
        // (SK8's sheet lists the shared layer's chords itself: its own menu's comes after them)
        app.world_mut().resource_mut::<crate::xr::ui::ControlsSheet>().set_whole(
            crate::engines::Engine::Sk8,
            menu::XR_CONTROLS.iter().map(|&(a, b)| engine_api::Control::new(a, b)).collect(),
        );
        app.add_systems(Startup, |panel: Res<crate::xr::panel::XrPanel>, mut commands: Commands| {
            commands.insert_resource(menu::MenuTarget(panel.camera));
        })
        .add_systems(
            Update,
            (
                xr_menu::touch_nav,
                menu::update,
                xr_menu::apply,
                input::read_touch,
                sim::apply_ar_world,
                sim::teleport,
                sim::step,
                sim::publish_link,
            )
                .chain()
                .after(crate::xr::ui::UiSet)
                .before(feed_sounds)
                .in_set(SK8),
        )
        .add_systems(Update, xr_menu::feed_hud.after(menu::draw).in_set(SK8));
        app.world_mut()
            .resource_mut::<crate::xr::ui::UiProviders>()
            .0
            .push(std::sync::Arc::new(skater_page::CleanPages));
    }
    if !xr {
        app.add_systems(Startup, view::desktop_world).add_systems(
            Update,
            (menu::keyboard_nav, menu::update, input::log_gamepads, input::read_desktop, input::demo_rail_gizmo, sim::step, view::chase, view::views)
                .chain()
                .before(feed_sounds)
                .in_set(SK8),
        );
    }
    app.init_resource::<skater::Clock>()
        .add_systems(Update, (feed_sounds.before(crate::skate_audio::clean::SkaterSoundSet), skater::bind, skater::animate, menu::draw).chain().in_set(SK8))
        .add_systems(
            PostUpdate,
            (mirror::apply, procedural::layer)
                .chain()
                .after(bevy::app::AnimationSystems)
                .before(bevy::transform::TransformSystems::Propagate)
                .in_set(SK8),
        );
}

/// This frame's steps as the skating sounds (the clean driver's).
fn feed_sounds(sim: Res<CleanSkate>, mut out: ResMut<crate::skate_audio::clean::SkaterSounds>) {
    use engine_core::skate::sound::{Cue, Rolling, SkateSounds};
    let s = &sim.current;
    let grounded = s.mode.grounded();
    let grinding = s.mode == Mode::Grind;
    let mut cues: Vec<Cue> = sim
        .events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::Pop => Some(Cue::Pop),
            EventKind::Land => Some(Cue::Land { impact: Vec3::from_array(s.velocity).length() }),
            EventKind::Push => Some(Cue::Push),
            EventKind::Bail => Some(Cue::Bail),
            EventKind::Bank => Some(Cue::Chime),
            _ => None,
        })
        .collect();
    // A flip starts when the trick name appears in the air.
    if s.flip_progress.is_some() && sim.previous.flip_progress.is_none() {
        cues.push(Cue::Flip);
    }
    *out = crate::skate_audio::clean::SkaterSounds {
        engine: Some(crate::engines::Engine::Sk8),
        paused: sim.paused,
        sounds: SkateSounds {
            at: Vec3::from_array(s.position),
            speed: s.speed,
            rolling: if grinding { Rolling::Grind } else if grounded { Rolling::Ground } else { Rolling::None },
            powersliding: s.mode == Mode::Slide,
            airborne: !grounded && !grinding,
            cues,
        },
    };
}
