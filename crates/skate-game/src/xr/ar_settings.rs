//! The player's AR settings (`ar-settings.json` in [`XrFiles`]): room or
//! free-roam mode, which kinds of real surface the skater rides and which
//! are drawn, the HUD, and whether the first-run intro has been seen.
//!
//! Every field has a default, so older or partial files still load.
//! `revision` changes on every edit: collision, the overlays and the room
//! scan read it, and it saves the file.
use super::XrFiles;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
pub(crate) use engine_api::HudHand;

const FILE: &str = "ar-settings.json";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ArMode {
    /// The Space Setup room: floor, walls, furniture and the room mesh.
    #[default]
    Room,
    /// Outside or anywhere unscanned: no room scan, just an endless floor
    /// under the skater (plus placed pieces).
    FreeRoam,
}

/// One kind of real surface: ride it, and draw it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Layer {
    #[serde(default = "yes")]
    pub collide: bool,
    #[serde(default)]
    pub show: bool,
}

impl Default for Layer {
    fn default() -> Self {
        Self { collide: true, show: false }
    }
}

fn yes() -> bool {
    true
}

fn one() -> f32 {
    1.0
}

#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ArSettings {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub onboarding_done: bool,
    #[serde(default)]
    pub mode: ArMode,
    /// Space Setup floors (or the fallback / free-roam floor).
    #[serde(default)]
    pub floor: Layer,
    /// Walls, table tops, furniture boxes and the flat patches found in the
    /// room mesh.
    #[serde(default)]
    pub planes: Layer,
    /// The scanned 3D room mesh (everything the planes don't cover).
    #[serde(default)]
    pub mesh: Layer,
    /// Free-roam floor height above STAGE y = 0, real metres.
    #[serde(default)]
    pub floor_offset: f32,
    /// The trick / points readout while skating: SK8's in the corner, Skate 3's and THUG2's own HUDs by a
    /// controller.
    #[serde(default = "yes")]
    pub hud: bool,
    /// Soft shadows under the board and round the park pieces (`xr::contact_shadow`).
    #[serde(default = "yes")]
    pub contact_shadows: bool,
    /// Which controller the games' own HUDs float by (`xr::hand_hud`).
    #[serde(default)]
    pub hud_hand: HudHand,
    /// The skater and the park lit by the real room (`xr::room_light`),
    /// which uses the headset's passthrough camera: off until asked for.
    #[serde(default)]
    pub room_light: bool,
    /// The room light's brightness against passthrough's (1: as estimated).
    #[serde(default = "one")]
    pub room_light_strength: f32,
    /// Build mode's legend beside the controllers (`xr::build::legend`).
    #[serde(default = "yes")]
    pub build_hints: bool,
    /// Build mode draws the room's surfaces that are ridden (the overlay).
    #[serde(default)]
    pub build_surfaces: bool,
    /// Real things hide the skater and the park (`xr::occlusion`): costs
    /// GPU time, so off until asked for.
    #[serde(default)]
    pub occlusion: super::occlusion::Occlusion,
    /// Occlusion's edges are found in the camera's image (`xr::occlusion_edges`):
    /// sharper and steadier, and it needs the headset's cameras.
    #[serde(default)]
    pub occlusion_edges: bool,
    /// Occlusion's edges are kept from frame to frame, so they hold still.
    #[serde(default)]
    pub occlusion_steady: bool,
    /// The hands and arms in occlusion's depth image are moved on with the
    /// controllers, so occlusion keeps up with them.
    #[serde(default)]
    pub occlusion_hands: bool,
    /// The controllers hide what is behind them by their own shapes
    /// (`xr::controller_shapes`): the depth image often misses them.
    #[serde(default)]
    pub occlusion_controllers: bool,
    /// Developer's pages in the menu (the sound lab). No row turns it on: it is set in the file.
    #[serde(default)]
    pub dev: bool,
    #[serde(skip)]
    pub revision: u64,
}

impl Default for ArSettings {
    fn default() -> Self {
        Self {
            version: 1,
            onboarding_done: false,
            mode: ArMode::Room,
            floor: Layer::default(),
            planes: Layer::default(),
            mesh: Layer::default(),
            floor_offset: 0.0,
            hud: true,
            hud_hand: HudHand::default(),
            contact_shadows: true,
            room_light: false,
            room_light_strength: 1.0,
            build_hints: true,
            build_surfaces: false,
            occlusion: Default::default(),
            occlusion_edges: false,
            occlusion_steady: false,
            occlusion_hands: false,
            occlusion_controllers: false,
            dev: false,
            revision: 0,
        }
    }
}

impl ArSettings {
    pub(crate) fn free_roam(&self) -> bool {
        self.mode == ArMode::FreeRoam
    }

    /// Any layer drawn outside build mode.
    pub(crate) fn any_shown(&self) -> bool {
        self.floor.show || self.planes.show || self.mesh.show
    }

    pub(crate) fn set_all_shown(&mut self, show: bool) {
        self.floor.show = show;
        self.planes.show = show;
        self.mesh.show = show;
    }
}

pub(crate) struct ArSettingsPlugin;

impl Plugin for ArSettingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ArSettings>()
            .add_systems(PreStartup, load)
            .add_systems(Last, save);
    }
}

fn load(files: Res<XrFiles>, mut settings: ResMut<ArSettings>) {
    let path = files.0.join(FILE);
    let Ok(bytes) = std::fs::read(&path) else { return };
    match serde_json::from_slice::<ArSettings>(&bytes) {
        Ok(loaded) => {
            *settings = ArSettings { revision: 1, ..loaded };
            info!("AR settings: loaded {} ({:?})", path.display(), settings.mode);
        }
        Err(e) => warn!("AR settings: ignoring {}: {e}", path.display()),
    }
}

fn save(files: Res<XrFiles>, settings: Res<ArSettings>, mut saved: Local<Option<u64>>) {
    if saved.is_none() {
        // Startup: the loaded file needs no rewrite.
        *saved = Some(settings.revision);
        return;
    }
    if *saved == Some(settings.revision) {
        return;
    }
    *saved = Some(settings.revision);
    let path = files.0.join(FILE);
    let result = serde_json::to_vec_pretty(&*settings)
        .map_err(|e| e.to_string())
        .and_then(|json| std::fs::write(&path, json).map_err(|e| e.to_string()));
    if let Err(e) = result {
        warn!("AR settings: could not save {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_and_partial_files_load_with_defaults() {
        let s: ArSettings = serde_json::from_str("{}").unwrap();
        assert!(s.floor.collide && s.planes.collide && s.mesh.collide);
        assert!(!s.any_shown());
        assert!(s.hud && !s.onboarding_done && !s.free_roam());
        assert!(!s.room_light && s.room_light_strength == 1.0);
        let s: ArSettings = serde_json::from_str(r#"{"mode":"FreeRoam","mesh":{"show":true}}"#).unwrap();
        assert!(s.free_roam() && s.mesh.show && s.mesh.collide);
    }

    #[test]
    fn round_trips() {
        let mut s = ArSettings { onboarding_done: true, floor_offset: 0.25, ..default() };
        s.planes.collide = false;
        let back: ArSettings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
