//! Skate 3 in the headset: the faithful port's simulation (`skate3_sim`, on the player's own imported data) as one of
//! the app's engines. What it knows of Skate 3 is `skate3_engine`'s (the sim as an `engine_core::EngineCore`:
//! `skate3_engine::engine`); the app's generic driver hosts it (`crate::engine_driver`: made on a worker when first
//! picked, ticked by its own period, its skater drawn from its GLB, its HUD, its sound).
//!
//! Here: where its data is (`--assets DIRECTORY` or `SKATE3_ASSET_ROOT` on the desktop, `assets/` in the app's files
//! in the headset, each with `private/game.json`), the asset source its GLB loads from, its light, which sound it plays
//! (its own graph on the decoded sound banks, else the skating sounds the app carries; `SKATE_AUDIO=clean|retail`),
//! and its settings and page.
use crate::engines::{self, Engine};
use bevy::prelude::*;
use std::path::{Path, PathBuf};

/// Skate 3 to the driver.
pub(crate) struct Skate3;

impl crate::engine_driver::Hosted for Skate3 {
    const ENGINE: Engine = Engine::Skate3;
}

type Driver = crate::engine_driver::Driver<Skate3>;

/// The asset source its data is ([`register_assets`]).
pub(crate) const ASSETS: &str = "skate3";

/// The player's imported Skate 3 data, if there is any: its `assets` folder.
pub(crate) fn assets(files: &Path) -> Option<PathBuf> {
    let mut given = std::env::var_os("SKATE3_ASSET_ROOT").map(PathBuf::from);
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--assets" {
            given = args.next().map(PathBuf::from);
        }
    }
    let root = given.unwrap_or_else(|| if cfg!(target_os = "android") { files.join("assets") } else { PathBuf::from("assets") });
    root.join("private/game.json").is_file().then_some(root)
}

/// Its data as an asset source (its GLB loads from it), before the app's plugins.
pub(crate) fn register_assets(app: &mut App, assets: &Path) {
    let path = assets.to_path_buf();
    app.register_asset_source(
        ASSETS,
        bevy::asset::io::AssetSourceBuilder::new(move || {
            Box::new(bevy::asset::io::file::FileAssetReader::new(path.clone())) as Box<dyn bevy::asset::io::ErasedAssetReader>
        }),
    );
}

/// Its own sound graph, unless this build plays the skating sounds the app carries instead: on an Android without the
/// decoded sound banks (the headset has no XMA decoder: they come with the imported data, `audio-pcm-cache`).
fn own_sound() -> bool {
    match std::env::var("SKATE_AUDIO").as_deref() {
        Ok("clean") => false,
        Ok("retail") => true,
        _ => !cfg!(target_os = "android") || skate3_engine::sound::banks_decoded(),
    }
}

fn available(world: &World) -> Result<(), String> {
    match world.get_resource::<Driver>().and_then(Driver::failed) {
        Some(why) => Err(why),
        None => Ok(()),
    }
}

/// Adds Skate 3 to the app's engines, its data at `assets` (`files`: the app's own folder).
pub(crate) fn install(app: &mut App, files: &Path, assets: PathBuf) {
    let difficulty = skate3_sim::difficulty::Difficulty::load(&assets).unwrap_or_default();
    let own = own_sound();
    info!("Skate 3 carried: {} ({}, {})", assets.display(), difficulty.key(), if own { "its own sound" } else { "the clean sounds" });
    let factory = skate3_engine::engine::factory(assets, difficulty, own);
    let _ = std::fs::create_dir_all(files);
    app.insert_resource(Settings::load(&files.join(SETTINGS_FILE)))
        .insert_resource(SettingsFile(files.join(SETTINGS_FILE)));
    app.world_mut().resource_mut::<crate::xr::ui::UiProviders>().0.push(std::sync::Arc::new(Skate3Pages));
    app.world_mut()
        .resource_mut::<crate::xr::ui::ControlsSheet>()
        .set(Engine::Skate3, skate3_engine::input::CONTROLS.to_vec());
    crate::engine_hud::install(app);
    app.init_resource::<engine_api::PlayerBodies>();
    engines::add_engine(app, Engine::Skate3, available, |app| {
        crate::engine_driver::install::<Skate3>(app, factory, Some(files.join(STARTING)), Some(ASSETS));
        app.add_systems(Startup, light).add_systems(Update, tune.in_set(engines::EngineSet(Engine::Skate3)));
    });
}

/// Made while Skate 3 starts, gone once it has.
const STARTING: &str = "skate3-starting";

/// Skate 3's light. The room light (`xr::room_light`) fades the sun and the ambient.
const CLEAR: Color = Color::srgb(0.065, 0.08, 0.10);
const AMBIENT: f32 = 350.;
const SUN_LUX: f32 = 11000.;

fn light(mut commands: Commands) {
    commands.insert_resource(ClearColor(CLEAR));
    commands.insert_resource(GlobalAmbientLight { color: Color::WHITE, brightness: AMBIENT, ..default() });
    commands.spawn((
        DirectionalLight { illuminance: SUN_LUX, shadows_enabled: true, ..default() },
        Transform::from_xyz(4., 7., 4.).looking_at(Vec3::ZERO, Vec3::Y),
        Name::new("Skate 3 sun"),
    ));
}

/// Skate 3's settings, kept in `skate3.json` beside the app's other settings.
#[derive(Resource, Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Settings {
    /// The trick display's multiplier circle always shown, not fading in and out with the line.
    circle_always: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { circle_always: true }
    }
}

const SETTINGS_FILE: &str = "skate3.json";

impl Settings {
    fn load(file: &Path) -> Self {
        std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
}

#[derive(Resource)]
struct SettingsFile(PathBuf);

/// Its settings to the engine (the driver gives them again each time it is made).
fn tune(settings: Res<Settings>, mut driver: ResMut<Driver>) {
    driver.set_setting(skate3_engine::engine::settings::CIRCLE_ALWAYS, if settings.circle_always { 1.0 } else { 0.0 });
}

/// Skate 3's page in the headset menu (shown while it runs).
struct Skate3Pages;

const PAGE: crate::xr::ui::PageId = "skate3";

impl crate::xr::ui::UiProvider for Skate3Pages {
    fn page(&self, id: crate::xr::ui::PageId, world: &World) -> Option<crate::xr::ui::UiPage> {
        use crate::xr::ui::{UiPage, Widget};
        if id != PAGE {
            return None;
        }
        let s = *world.resource::<Settings>();
        Some(UiPage::new(
            "Skate 3",
            vec![
                Widget::toggle("circle_always", "HUD: always show the multiplier circle", s.circle_always),
                Widget::info("Off, it fades in and out with each line, as the game has it."),
            ],
        ))
    }

    fn act(
        &self,
        page: crate::xr::ui::PageId,
        widget: &'static str,
        act: crate::xr::ui::Act,
        world: &mut World,
    ) -> Option<crate::xr::ui::Outcome> {
        use crate::xr::ui::{Act, Outcome};
        if page != PAGE {
            return None;
        }
        let mut s = *world.resource::<Settings>();
        match (widget, act) {
            ("circle_always", Act::Press) => s.circle_always = !s.circle_always,
            _ => return Some(Outcome::Stay),
        }
        world.insert_resource(s);
        let file = world.resource::<SettingsFile>().0.clone();
        if let Ok(json) = serde_json::to_vec_pretty(&s) {
            let _ = std::fs::write(&file, json);
        }
        Some(Outcome::Stay)
    }

    fn start_items(&self, _world: &World) -> Vec<crate::xr::ui::Widget> {
        vec![crate::xr::ui::Widget::link("Skate 3: HUD", PAGE)]
    }

    fn engine(&self) -> Option<Engine> {
        Some(Engine::Skate3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    /// Its light: 11,000 lux with shadows from (4, 7, 4) at the origin, ambient 350 and its clear colour.
    #[test]
    fn skate3_keeps_its_light() {
        let mut world = World::new();
        world.run_system_once(light).unwrap();
        let lights: Vec<_> = world
            .query::<(&DirectionalLight, &Transform)>()
            .iter(&world)
            .map(|(l, t)| (l.illuminance, l.shadows_enabled, l.color, *t))
            .collect();
        assert_eq!(lights, [(11000., true, Color::WHITE, Transform::from_xyz(4., 7., 4.).looking_at(Vec3::ZERO, Vec3::Y))]);
        assert_eq!(world.resource::<GlobalAmbientLight>().brightness, 350.);
        assert_eq!(world.resource::<ClearColor>().0, Color::srgb(0.065, 0.08, 0.10));
    }

    /// The circle always shown unless the player turned it off; files from before it existed load with it on.
    #[test]
    fn the_circle_is_shown_by_default() {
        assert!(Settings::default().circle_always);
        let dir = std::env::temp_dir().join(format!("skate3-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("old.json"), b"{}").unwrap();
        assert!(Settings::load(&dir.join("old.json")).circle_always);
        std::fs::write(dir.join("off.json"), br#"{ "circle_always": false }"#).unwrap();
        assert!(!Settings::load(&dir.join("off.json")).circle_always);
        let _ = std::fs::remove_dir_all(dir);
    }
}
