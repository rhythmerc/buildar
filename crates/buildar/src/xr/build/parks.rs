//! Saved parks: each one a file in `parks/` next to the assets folder (on
//! Quest, the app's external files directory), opened from the parks screen
//! (`xr::ui::parks`).
//!
//! A park's pieces are kept in its own frame, in game metres: so a park
//! keeps the size it was made at against the skater wherever it is put,
//! and the same park can be put at a different size in another place. Where
//! the park is in a room is kept by spatial anchors ([`super::anchor`]): each
//! of its anchors says where the park's frame is from it, and at what world
//! scale (game metres per real metre: the skater's size) it was put there.
//! A park found by none of its anchors in this room is put down by hand
//! ([`Placing`]), and gets a new anchor there; its old ones stay, so it opens
//! in either place.
//!
//! While a park is open its pieces are in the tracking space, as everything
//! else in build mode is ([`PlacedPieces`]), and the park's frame is
//! [`Current::pose`] there.
use super::anchor::Anchors;
use super::{MAX_PARK_SCALE, MAX_SCALE, MIN_PARK_SCALE, MIN_SCALE, Placed, PlacedPieces};
use crate::engines::{CourseSettings, CourseStepper, CourseSteppers, Engine, Engines, Mode};
use crate::xr::rig::Rig;
use crate::xr::XrFiles;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const VERSION: u32 = 2;
/// The layout file from before there were parks: it becomes the first park.
const OLD_LAYOUT: &str = "ar-layout.json";
const DIR: &str = "parks";

/// One of a park's places: a spatial anchor, and where the park's frame is
/// from it.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub(crate) struct AnchorRecord {
    /// The anchor's UUID, 32 hex digits.
    pub uuid: String,
    /// The park frame's origin in the anchor's frame, real metres.
    pub offset: [f32; 3],
    /// The park frame's turn less the anchor's, about +Y.
    pub turn: f32,
    /// The world scale the park was put at here.
    pub scale: f32,
    /// When it was put here (seconds since 1970): where a park is found by
    /// two anchors at once, the newer place wins.
    #[serde(default)]
    pub placed: u64,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub(crate) struct ParkFile {
    pub version: u32,
    pub name: String,
    /// Seconds since 1970.
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub opened: u64,
    /// In the park's own frame, game metres (yaw from the frame's).
    pub pieces: Vec<Placed>,
    /// The world scale it was last open at: a new place starts at it.
    pub scale: f32,
    pub park_scale: f32,
    #[serde(default)]
    pub anchors: Vec<AnchorRecord>,
    /// The engine it was built for (`Engine::key`: "sk8", "skate3", "thug2"); none: a park from before engines were
    /// kept, played with whichever runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// That engine's settings it was built with (`engines::CourseSettings`: THUG2's game speed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_settings: Option<serde_json::Value>,
    /// The mode it is a course of (`Mode::key`: "skate"); a park from before modes were kept is a skate park. Kept
    /// as written, so a course of a mode this app doesn't know is recognised as such and left alone.
    #[serde(default = "skate_mode")]
    pub mode: String,
}

/// A park's mode tag.
fn skate_mode() -> String {
    Mode::Skate.key().to_string()
}

impl ParkFile {
    /// The mode it is a course of, if this app knows it.
    pub(crate) fn mode(&self) -> Option<Mode> {
        Mode::from_key(&self.mode)
    }

    /// The engine it was built for, if this app knows it.
    pub(crate) fn engine(&self) -> Option<Engine> {
        self.engine.as_deref().and_then(Engine::from_key)
    }
}

/// A frame on the floor of the tracking space: where its origin is, and its
/// turn about +Y.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub(crate) struct ParkPose {
    pub at: Vec3,
    pub yaw: f32,
}

impl ParkPose {
    /// A park-frame point (game metres) in the tracking space, at world scale `scale`.
    pub(crate) fn to_room(&self, point: Vec3, scale: f32) -> Vec3 {
        self.at + Quat::from_rotation_y(self.yaw) * (point / scale)
    }

    /// A tracking-space point in the park's frame, game metres.
    pub(crate) fn to_park(&self, point: Vec3, scale: f32) -> Vec3 {
        Quat::from_rotation_y(-self.yaw) * (point - self.at) * scale
    }

    /// Where the park's frame is, by an anchor at `at` turned `yaw`.
    pub(crate) fn by_anchor(at: Vec3, yaw: f32, record: &AnchorRecord) -> Self {
        Self { at: at + Quat::from_rotation_y(yaw) * Vec3::from_array(record.offset), yaw: yaw + record.turn }
    }

    /// The record an anchor at `at` turned `yaw` keeps of this frame.
    pub(crate) fn record(&self, uuid: String, at: Vec3, yaw: f32, scale: f32, placed: u64) -> AnchorRecord {
        AnchorRecord { uuid, offset: (Quat::from_rotation_y(-yaw) * (self.at - at)).to_array(), turn: self.yaw - yaw, scale, placed }
    }
}

/// A park's pieces from its file, put in the tracking space by `pose` at `scale`.
pub(crate) fn in_room(pieces: &[Placed], pose: ParkPose, scale: f32) -> Vec<Placed> {
    pieces
        .iter()
        .map(|p| Placed { position: pose.to_room(Vec3::from_array(p.position), scale).to_array(), yaw: p.yaw + pose.yaw, ..p.clone() })
        .collect()
}

/// The pieces in the tracking space, as a park's file keeps them.
pub(crate) fn in_park(items: &[Placed], pose: ParkPose, scale: f32) -> Vec<Placed> {
    items
        .iter()
        .map(|p| Placed { position: pose.to_park(Vec3::from_array(p.position), scale).to_array(), yaw: p.yaw - pose.yaw, ..p.clone() })
        .collect()
}

/// The pieces moved with their frame, from `from` to `to`.
pub(crate) fn moved(items: &mut [Placed], from: ParkPose, to: ParkPose) {
    let turn = to.yaw - from.yaw;
    let rotation = Quat::from_rotation_y(turn);
    for item in items {
        let offset = Vec3::from_array(item.position) - from.at;
        item.position = (to.at + rotation * offset).to_array();
        item.yaw += turn;
    }
}

/// Where a park is held when it is put down: under the middle of its pieces,
/// at the lowest of them (park frame, game metres).
pub(crate) fn pivot(pieces: &[Placed]) -> Vec3 {
    if pieces.is_empty() {
        return Vec3::ZERO;
    }
    let (lo, hi) = pieces.iter().map(|p| Vec3::from_array(p.position)).fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)));
    Vec3::new((lo.x + hi.x) * 0.5, lo.y, (lo.z + hi.z) * 0.5)
}

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

// ---------------------------------------------------------------- files

fn dir(files: &Path) -> PathBuf {
    files.join(DIR)
}

fn file_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// Every park in `dir`, as (id, file); unreadable files are left out.
fn read_all(dir: &Path) -> Vec<(String, ParkFile)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
        match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice::<ParkFile>(&b).map_err(|e| e.to_string())) {
            Ok(file) if file.version == VERSION && file.mode().is_some() => out.push((id, file)),
            Ok(file) if file.version == VERSION => warn!("PARKS ignoring {} (a {:?} course)", path.display(), file.mode),
            Ok(file) => warn!("PARKS ignoring {} (version {})", path.display(), file.version),
            Err(e) => warn!("PARKS ignoring {}: {e}", path.display()),
        }
    }
    out
}

/// Written beside it, then moved over it, so a crash never leaves half a park.
fn write(dir: &Path, id: &str, file: &ParkFile) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_vec_pretty(file).map_err(|e| e.to_string())?;
    let path = file_path(dir, id);
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, json).map_err(|e| e.to_string())?;
    std::fs::rename(&temporary, &path).map_err(|e| e.to_string())
}

/// The layout file from before parks (version 1).
#[derive(Deserialize)]
struct OldLayout {
    version: u32,
    pieces: Vec<Placed>,
    #[serde(default)]
    scale: Option<f32>,
    #[serde(default)]
    park_scale: Option<f32>,
    #[serde(default)]
    anchor: Option<OldAnchor>,
}

#[derive(Deserialize)]
struct OldAnchor {
    uuid: String,
    position: [f32; 3],
    yaw: f32,
}

/// The old layout as a park: its pieces were in the tracking space, so the
/// park's frame is the tracking space's as it was, and its anchor (where it
/// was in that space) says where that frame is from it.
fn from_old(old: OldLayout, default_scale: f32, at: u64) -> Option<ParkFile> {
    if old.version != 1 {
        return None;
    }
    let scale = old.scale.filter(|s| (MIN_SCALE..=MAX_SCALE).contains(s)).unwrap_or(default_scale);
    let anchors = old
        .anchor
        .map(|a| ParkPose::default().record(a.uuid, Vec3::from_array(a.position), a.yaw, scale, at))
        .into_iter()
        .collect();
    Some(ParkFile {
        version: VERSION,
        name: "My park".into(),
        created: at,
        opened: at,
        pieces: in_park(&old.pieces, ParkPose::default(), scale),
        scale,
        park_scale: old.park_scale.filter(|s| (MIN_PARK_SCALE..=MAX_PARK_SCALE).contains(s)).unwrap_or(1.0),
        anchors,
        engine: None,
        engine_settings: None,
        mode: skate_mode(),
    })
}

/// A name no other park has: "Park 2", "Park 3", ...
fn next_name(parks: &[ParkEntry], stem: &str) -> String {
    (1..).map(|n| if n == 1 && stem != "Park" { stem.to_string() } else { format!("{stem} {n}") }).find(|name| !parks.iter().any(|p| &p.file.name == name)).expect("a free name")
}

fn new_id(parks: &[ParkEntry]) -> String {
    let base = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    (0..).map(|n| format!("{:x}", base + n)).find(|id| !parks.iter().any(|p| &p.id == id)).expect("a free id")
}

// ---------------------------------------------------------------- runtime

#[derive(Clone, Debug)]
pub(crate) struct ParkEntry {
    pub id: String,
    pub file: ParkFile,
    /// Changes whenever the park is saved (its card's picture is drawn again).
    pub edits: u64,
}

/// The park that is open.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Current {
    pub id: String,
    /// Its frame in the tracking space.
    pub pose: ParkPose,
    /// The anchor it is kept by in this room (none yet: one is made).
    pub anchor: Option<String>,
}

#[derive(Resource, Default)]
pub(crate) struct Parks {
    pub list: Vec<ParkEntry>,
    pub current: Option<Current>,
    dir: Option<PathBuf>,
    /// Changes when the list does (a park added, removed, renamed, opened, saved).
    pub revision: u64,
}

/// Whether a park is in this room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Here {
    /// Found by an anchor: it opens where it was.
    Found,
    /// The headset is still looking for the anchors.
    Looking,
    /// Not found here: it has to be put down.
    Away,
}

impl Parks {
    pub(crate) fn get(&self, id: &str) -> Option<&ParkEntry> {
        self.list.iter().find(|p| p.id == id)
    }

    pub(crate) fn get_mut(&mut self, id: &str) -> Option<&mut ParkEntry> {
        self.list.iter_mut().find(|p| p.id == id)
    }

    pub(crate) fn is_current(&self, id: &str) -> bool {
        self.current.as_ref().is_some_and(|c| c.id == id)
    }

    /// The anchor `id` is found by here (the newest place, if several), and where it is.
    pub(crate) fn found_by(&self, id: &str, anchors: &Anchors) -> Option<(AnchorRecord, Vec3, f32)> {
        let park = self.get(id)?;
        park.file
            .anchors
            .iter()
            .filter_map(|r| anchors.located(&r.uuid).map(|(at, yaw)| (r.clone(), at, yaw)))
            .max_by_key(|(r, ..)| r.placed)
    }

    pub(crate) fn here(&self, id: &str, anchors: &Anchors) -> Here {
        // (the open park is here: it is open in this room, anchored yet or not)
        if self.is_current(id) || self.found_by(id, anchors).is_some() {
            Here::Found
        } else if anchors.searching() {
            Here::Looking
        } else {
            Here::Away
        }
    }

    /// The order of the cards: parks here first, then the rest; each by when it was last opened.
    pub(crate) fn sorted(&self, anchors: &Anchors) -> Vec<(usize, Here)> {
        let mut order: Vec<(usize, Here)> = self.list.iter().enumerate().map(|(i, p)| (i, self.here(&p.id, anchors))).collect();
        order.sort_by(|a, b| {
            let rank = |h: Here| if h == Here::Found { 0 } else { 1 };
            rank(a.1).cmp(&rank(b.1)).then(self.list[b.0].file.opened.cmp(&self.list[a.0].file.opened)).then(self.list[a.0].file.name.cmp(&self.list[b.0].file.name))
        });
        order
    }

    pub(crate) fn save(&mut self, id: &str) {
        // (a course the dev flag `course=` opened in its debug room is never written: `xr::debug_course`)
        if self.current.as_ref().is_some_and(|c| c.id == id && c.anchor.as_deref() == Some(DEBUG_ANCHOR)) {
            return;
        }
        let (Some(dir), Some(entry)) = (self.dir.clone(), self.get_mut(id)) else { return };
        entry.edits += 1;
        if let Err(e) = write(&dir, id, &entry.file) {
            warn!("PARKS could not save {id}: {e}");
        }
        self.revision += 1;
    }
}

/// A park being put down: its pieces follow the pointer until it is put.
#[derive(Clone, Debug)]
pub(crate) struct PlaceJob {
    pub id: String,
    /// Build it, once put (else skate it).
    pub edit: bool,
    /// Its pieces as its file keeps them, and where it is held.
    pub pieces: Vec<Placed>,
    pub pivot: Vec3,
    pub yaw: f32,
    /// The world scale it is being put at, and its engine's own (x1.0: `size`).
    pub scale: f32,
    pub default: f32,
    /// Where it is now (tracking space), if the pointer is on a surface.
    pub at: Option<Vec3>,
    /// What was open before, to go back to if it is let go.
    pub before: Option<(Current, Vec<Placed>, f32, f32)>,
    pub turned: bool,
}

#[derive(Resource, Default)]
pub(crate) struct Placing(pub Option<PlaceJob>);

impl PlaceJob {
    /// The park's frame when its pivot is at `at`.
    pub(crate) fn pose(&self, at: Vec3) -> ParkPose {
        ParkPose { at: at - Quat::from_rotation_y(self.yaw) * (self.pivot / self.scale), yaw: self.yaw }
    }
}

pub(super) struct ParksPlugin;

impl Plugin for ParksPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Parks>()
            .init_resource::<Placing>()
            .add_systems(Startup, load)
            .add_systems(Update, (take_made_anchors, follow, save, make_anchor, tag_engine).chain());
    }
}

/// The parks on disk (the old layout first made one).
fn load(files: Res<XrFiles>, rig: Res<Rig>, mut parks: ResMut<Parks>) {
    let dir = dir(&files.0);
    let mut found = read_all(&dir);
    let old = files.0.join(OLD_LAYOUT);
    if found.is_empty() && old.exists() {
        match std::fs::read(&old).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice::<OldLayout>(&b).map_err(|e| e.to_string())) {
            Ok(layout) => {
                if let Some(file) = from_old(layout, rig.scale, now()) {
                    let id = new_id(&[]);
                    match write(&dir, &id, &file) {
                        Ok(()) => {
                            info!("PARKS the old layout ({} pieces) is now the park {id}", file.pieces.len());
                            let _ = std::fs::rename(&old, old.with_extension("json.before-parks"));
                            found.push((id, file));
                        }
                        Err(e) => warn!("PARKS could not keep the old layout as a park: {e}"),
                    }
                }
            }
            Err(e) => warn!("PARKS ignoring {}: {e}", old.display()),
        }
    }
    info!("PARKS {} saved in {}", found.len(), dir.display());
    parks.list = found.into_iter().map(|(id, file)| ParkEntry { id, file, edits: 0 }).collect();
    parks.dir = Some(dir);
    parks.revision += 1;
}

/// The open park is written on every committed edit (not while a piece is
/// being dragged).
fn save(placed: Res<PlacedPieces>, rig: Res<Rig>, placing: Res<Placing>, mut parks: ResMut<Parks>, mut saved: Local<Option<(String, u64)>>) {
    // Pieces built with no park open make a new park.
    if parks.current.is_none() && placing.0.is_none() && !placed.items.is_empty() && !placed.busy() && parks.dir.is_some() {
        let (id, at) = (new_id(&parks.list), now());
        let name = next_name(&parks.list, "Park");
        let file = ParkFile { version: VERSION, name, created: at, opened: at, pieces: in_park(&placed.items, ParkPose::default(), rig.scale), scale: rig.scale, park_scale: placed.park_scale, anchors: Vec::new(), engine: None, engine_settings: None, mode: skate_mode() };
        parks.list.push(ParkEntry { id: id.clone(), file, edits: 0 });
        parks.current = Some(Current { id: id.clone(), pose: ParkPose::default(), anchor: None });
        parks.save(&id);
        info!("PARKS {} pieces built with no park open: the new park {id}", placed.items.len());
    }
    let Some(current) = parks.current.clone() else {
        *saved = None;
        return;
    };
    if placed.busy() || placing.0.is_some() {
        return;
    }
    let key = (current.id.clone(), placed.revision);
    if saved.as_ref() == Some(&key) {
        return;
    }
    // (just opened: it is as its file has it)
    let first = saved.as_ref().is_none_or(|(id, _)| *id != current.id);
    *saved = Some(key);
    if first {
        return;
    }
    let Some(entry) = parks.get_mut(&current.id) else { return };
    entry.file.pieces = in_park(&placed.items, current.pose, rig.scale);
    entry.file.scale = rig.scale;
    entry.file.park_scale = placed.park_scale;
    // (its place here is at the scale it is now)
    if let Some(record) = current.anchor.as_ref().and_then(|u| entry.file.anchors.iter_mut().find(|r| &r.uuid == u)) {
        record.scale = rig.scale;
    }
    parks.save(&current.id);
}

/// The anchor the open park is kept by moved in the tracking space (the
/// first time it is found, or the boundary was re-centred): the pieces move
/// with it.
fn follow(anchors: Res<Anchors>, mut placed: ResMut<PlacedPieces>, mut parks: ResMut<Parks>) {
    // (pieces are not moved under a hand; mid-grind they are, and the engine takes the new stage when the grind ends)
    if placed.busy() {
        return;
    }
    let Some(current) = parks.current.clone() else { return };
    let Some(uuid) = current.anchor.clone() else { return };
    let Some((at, yaw)) = anchors.located(&uuid) else { return };
    let Some(record) = parks.get(&current.id).and_then(|p| p.file.anchors.iter().find(|r| r.uuid == uuid)).cloned() else { return };
    let to = ParkPose::by_anchor(at, yaw, &record);
    let (moved_m, turned) = (to.at.distance(current.pose.at), wrapped(to.yaw - current.pose.yaw).abs());
    if moved_m < MOVED_M && turned < TURNED_RAD {
        return;
    }
    info!("PARKS the anchor moved {:.1} cm and turned {:.2} deg: the pieces with it", moved_m * 100.0, turned.to_degrees());
    moved(&mut placed.items, current.pose, to);
    placed.revision += 1;
    if let Some(c) = parks.current.as_mut() {
        c.pose = to;
    }
}

/// The anchor moved this far (m), or turned this far, since the pieces were
/// last put by it: they are put by it again. (Under it: tracking noise.)
const MOVED_M: f32 = 0.01;
const TURNED_RAD: f32 = 0.5 * std::f32::consts::PI / 180.0;

fn wrapped(angle: f32) -> f32 {
    (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// The open park has pieces and no anchor in this room: one is made, in the
/// middle of it.
fn make_anchor(placed: Res<PlacedPieces>, placing: Res<Placing>, parks: Res<Parks>, mut anchors: ResMut<Anchors>) {
    let Some(current) = parks.current.as_ref() else { return };
    if current.anchor.is_some() || placed.items.is_empty() || placed.busy() || placing.0.is_some() || !anchors.can_make() {
        return;
    }
    let count = placed.items.len() as f32;
    let middle = placed.items.iter().map(|i| Vec3::from_array(i.position)).sum::<Vec3>() / count;
    let lowest = placed.items.iter().map(|i| i.position[1]).fold(f32::MAX, f32::min);
    anchors.make(current.id.clone(), Vec3::new(middle.x, lowest, middle.z));
}

/// A made anchor joins the park it was made for (if that is still the open one).
fn take_made_anchors(rig: Res<Rig>, mut anchors: ResMut<Anchors>, mut parks: ResMut<Parks>) {
    for (park, uuid, at, yaw) in anchors.take_made() {
        let Some(current) = parks.current.clone().filter(|c| c.id == park && c.anchor.is_none()) else {
            info!("PARKS an anchor made for {park}, which isn't open now: let go");
            anchors.erase(&uuid);
            continue;
        };
        let record = current.pose.record(uuid.clone(), at, yaw, rig.scale, now());
        if let Some(entry) = parks.get_mut(&park) {
            entry.file.anchors.push(record);
        }
        if let Some(c) = parks.current.as_mut() {
            c.anchor = Some(uuid);
        }
        parks.save(&park);
    }
}

/// The open park keeps the engine it's played with, and that engine's park settings, whenever they change (an
/// engine switched to, its game speed changed). Not while a switch is pending, nor when the park's own engine isn't
/// installed (played with another for now: it stays the one it was built for), nor by an engine of another mode.
pub(crate) fn tag_engine(world: &mut World) {
    let Some(engine) = world.get_resource::<Engines>().and_then(Engines::settled) else { return };
    if world.resource::<Placing>().0.is_some() {
        return;
    }
    let Some(id) = world.resource::<Parks>().current.as_ref().map(|c| c.id.clone()) else { return };
    let Some(file) = world.resource::<Parks>().get(&id).map(|p| p.file.clone()) else { return };
    if file.mode() != Some(engine.mode()) {
        return;
    }
    if let Some(built) = file.engine().filter(|&e| e != engine) {
        if world.resource::<Engines>().available(built, world).is_err() {
            return;
        }
    }
    let key = Some(engine.key().to_string());
    let settings = CourseSettings::of(world, engine);
    if file.engine == key && file.engine_settings == settings {
        return;
    }
    let mut parks = world.resource_mut::<Parks>();
    if let Some(entry) = parks.get_mut(&id) {
        entry.file.engine = key;
        entry.file.engine_settings = settings;
    }
    parks.save(&id);
    info!("PARKS {id} is played with {}", engine.name());
}

/// How park `id`'s engine fits the app now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EngineFit {
    /// Its engine runs (or it has none): it opens as it is.
    Running,
    /// Its engine is installed but another runs: opening it switches.
    Switch(Engine),
    /// Its engine isn't installed: it can only be played with the one running.
    Missing(Engine),
}

pub(crate) fn engine_fit(world: &World, id: &str) -> EngineFit {
    let built = world.resource::<Parks>().get(id).and_then(|p| p.file.engine());
    let Some(engines) = world.get_resource::<Engines>() else { return EngineFit::Running };
    // (one the menus don't offer, as SK8 to players: as if it had none)
    match built.filter(|&e| engines.listed(e, world)) {
        Some(e) if engines.available(e, world).is_err() => EngineFit::Missing(e),
        Some(e) if engines.current() != Some(e) => EngineFit::Switch(e),
        _ => EngineFit::Running,
    }
}

/// Park `id`'s engine switched to (when it's installed) and its settings put back, before it's opened (one that kept
/// none: the engine's defaults where it has them). A course
/// built for none, of another mode than the running engine's: the first engine of its mode that can run.
pub(crate) fn use_its_engine(world: &mut World, id: &str) {
    let Some(file) = world.resource::<Parks>().get(id).map(|p| p.file.clone()) else { return };
    let Some(engine) = file.engine().or_else(|| engine_of_mode(world, file.mode()?)) else { return };
    if world.get_resource::<Engines>().is_none_or(|e| !e.listed(engine, world) || e.available(engine, world).is_err()) {
        return;
    }
    if world.resource::<Engines>().chosen() != Some(engine) {
        world.resource_mut::<Engines>().request(engine);
        info!("PARKS {id} was built for {}: switching to it", engine.name());
    }
    // (a course that kept none: `{}`, each engine's own defaults for what it fills in - else the last course's
    // would carry over)
    let settings = file.engine_settings.clone().unwrap_or_else(|| serde_json::json!({}));
    CourseSettings::apply(world, engine, &settings);
}

/// The engine course `id` plays on: the one it was built for, else its mode's.
pub(crate) fn course_engine(world: &World, id: &str) -> Option<Engine> {
    let file = &world.resource::<Parks>().get(id)?.file;
    file.engine().or_else(|| engine_of_mode(world, file.mode()?))
}

/// The settings course `id` keeps that the player steps (its engine's, in the course settings dialog).
pub(crate) fn course_steppers(world: &World, id: &str) -> Vec<CourseStepper> {
    let Some(engine) = course_engine(world, id) else { return Vec::new() };
    world.get_resource::<CourseSteppers>().and_then(|s| s.0.get(&engine)).cloned().unwrap_or_default()
}

/// A stepped setting's value on course `id`: what it keeps, else its default.
pub(crate) fn stepper_value(world: &World, id: &str, stepper: &CourseStepper) -> f32 {
    let kept = world.resource::<Parks>().get(id).and_then(|p| p.file.engine_settings.as_ref()?.get(stepper.key)?.as_f64());
    kept.map_or(stepper.default, |v| v as f32)
}

/// A stepped setting of course `id` set: kept with it, and in play now if it's the open course.
pub(crate) fn set_stepper(world: &mut World, id: &str, stepper: &CourseStepper, value: f32) {
    let Some(engine) = course_engine(world, id) else { return };
    let settings = {
        let mut parks = world.resource_mut::<Parks>();
        let Some(entry) = parks.get_mut(id) else { return };
        let mut settings = entry.file.engine_settings.take().filter(serde_json::Value::is_object).unwrap_or_else(|| serde_json::json!({}));
        settings[stepper.key] = serde_json::json!(value);
        entry.file.engine_settings = Some(settings.clone());
        parks.save(id);
        settings
    };
    if world.resource::<Parks>().is_current(id) {
        CourseSettings::apply(world, engine, &settings);
    }
}

/// For a course of `mode`: the running engine if it is of that mode, else the first of that mode the menus offer
/// that can run.
pub(crate) fn engine_of_mode(world: &World, mode: Mode) -> Option<Engine> {
    let engines = world.get_resource::<Engines>()?;
    if engines.chosen().is_some_and(|e| e.mode() == mode) {
        return engines.chosen();
    }
    Engine::ALL.into_iter().find(|&e| e.mode() == mode && engines.listed(e, world) && engines.available(e, world).is_ok())
}

// ---------------------------------------------------------------- actions (the parks screen's)

/// The pieces, scales and park of `file` made the open park, by `pose`.
fn open_as(world: &mut World, id: &str, file: &ParkFile, pose: ParkPose, scale: f32, anchor: Option<String>) {
    let scale = scale.clamp(MIN_SCALE, MAX_SCALE);
    world.resource_mut::<Rig>().scale = scale;
    {
        let mut placed = world.resource_mut::<PlacedPieces>();
        placed.replace(in_room(&file.pieces, pose, scale));
        placed.park_scale = file.park_scale.clamp(MIN_PARK_SCALE, MAX_PARK_SCALE);
    }
    let mut parks = world.resource_mut::<Parks>();
    parks.current = Some(Current { id: id.to_string(), pose, anchor });
    if let Some(entry) = parks.get_mut(id) {
        entry.file.opened = now();
    }
    parks.save(id);
    info!("PARKS opened {id} ({} pieces, world scale {scale:.2})", file.pieces.len());
}

/// The anchor a course opened by `xr::debug_course` is "kept by": none in the room (no anchor is made for it, and it
/// is never saved).
pub(crate) const DEBUG_ANCHOR: &str = "debug-room";

/// Opens park `id` for measuring (`xr::debug_course`): its middle at `target` (tracking space), turned `yaw`, kept
/// by no anchor and never written; the player dropped onto its middle. Returns the park's id if there is one.
pub(crate) fn open_for_debug(world: &mut World, id: &str, target: Vec3, yaw: f32) -> bool {
    let Some(file) = world.resource::<Parks>().get(id).map(|p| p.file.clone()) else { return false };
    let scale = file.scale.clamp(MIN_SCALE, MAX_SCALE);
    let middle = pivot(&file.pieces);
    let pose = ParkPose { at: target - Quat::from_rotation_y(yaw) * (middle / scale), yaw };
    world.resource_mut::<Rig>().scale = scale;
    {
        let mut placed = world.resource_mut::<PlacedPieces>();
        placed.replace(in_room(&file.pieces, pose, scale));
        placed.park_scale = file.park_scale.clamp(MIN_PARK_SCALE, MAX_PARK_SCALE);
    }
    world.resource_mut::<Parks>().current = Some(Current { id: id.to_string(), pose, anchor: Some(DEBUG_ANCHOR.into()) });
    info!("PARKS opened {id} for measuring ({} pieces, world scale {scale:.2}) at {target:.2?}", file.pieces.len());
    true
}

/// The parks, for picking one by name: (id, name, pieces).
pub(crate) fn listed(world: &World) -> Vec<(String, String, usize)> {
    world.resource::<Parks>().list.iter().map(|p| (p.id.clone(), p.file.name.clone(), p.file.pieces.len())).collect()
}

/// Opens park `id` where one of its anchors has it.
pub(crate) fn open_here(world: &mut World, id: &str) -> bool {
    let found = world.resource::<Parks>().found_by(id, world.resource::<Anchors>());
    let (Some((record, at, yaw)), Some(file)) = (found, world.resource::<Parks>().get(id).map(|p| p.file.clone())) else { return false };
    let pose = ParkPose::by_anchor(at, yaw, &record);
    // (its player isn't put on it: the player places it - `engine_driver`, the loading card)
    open_as(world, id, &file, pose, record.scale, Some(record.uuid));
    true
}

/// Starts putting park `id` down by hand: its pieces follow the pointer.
pub(crate) fn start_placing(world: &mut World, id: &str, edit: bool) -> bool {
    let Some(file) = world.resource::<Parks>().get(id).map(|p| p.file.clone()) else { return false };
    let before = {
        let parks = world.resource::<Parks>();
        let placed = world.resource::<PlacedPieces>();
        parks.current.clone().map(|c| (c, placed.items.clone(), world.resource::<Rig>().scale, placed.park_scale))
    };
    let yaw = world.resource::<Rig>().anchor.map_or(0.0, |(_, yaw)| yaw);
    let scale = file.scale.clamp(MIN_SCALE, MAX_SCALE);
    world.resource_mut::<Rig>().scale = scale;
    {
        let mut placed = world.resource_mut::<PlacedPieces>();
        placed.replace(Vec::new());
        placed.park_scale = file.park_scale.clamp(MIN_PARK_SCALE, MAX_PARK_SCALE);
    }
    world.resource_mut::<Parks>().current = None;
    let default = super::size::default_of(world, &file);
    world.resource_mut::<Placing>().0 = Some(PlaceJob { id: id.to_string(), edit, pivot: pivot(&file.pieces), pieces: file.pieces, yaw, scale, default, at: None, before, turned: false });
    info!("PARKS putting {id} down");
    true
}

/// The park being put down is put where it is now.
pub(crate) fn finish_placing(world: &mut World) -> Option<bool> {
    let job = world.resource::<Placing>().0.clone()?;
    let at = job.at?;
    let pose = job.pose(at);
    let file = world.resource::<Parks>().get(&job.id)?.file.clone();
    world.resource_mut::<Placing>().0 = None;
    // (a new anchor is made for it here: `make_anchor`)
    open_as(world, &job.id, &file, pose, job.scale, None);
    info!("PARKS put {} down at {at:.2?}, world scale {:.2}", job.id, job.scale);
    Some(job.edit)
}

/// The park being put down is let go: what was open before comes back.
pub(crate) fn cancel_placing(world: &mut World) {
    let Some(job) = world.resource_mut::<Placing>().0.take() else { return };
    match job.before {
        Some((current, items, scale, park_scale)) => {
            world.resource_mut::<Rig>().scale = scale;
            let mut placed = world.resource_mut::<PlacedPieces>();
            placed.replace(items);
            placed.park_scale = park_scale;
            world.resource_mut::<Parks>().current = Some(current);
        }
        None => world.resource_mut::<PlacedPieces>().replace(Vec::new()),
    }
    info!("PARKS putting {} down let go", job.id);
}

/// A new, empty park, open here (its anchor is made with its first piece).
/// What a course of `mode` is called (its new card's, its default name): a skate park, a race track.
pub(crate) fn course_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Skate => "Park",
        Mode::Race => "Track",
    }
}

/// Courses of `mode`, as a tab says it.
pub(crate) fn courses_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Skate => "Parks",
        Mode::Race => "Tracks",
    }
}

/// A new course of `mode`, empty, opened here to build.
pub(crate) fn new_park(world: &mut World, mode: Mode) -> String {
    // (at its engine's own size, x1.0)
    let scale = engine_of_mode(world, mode).map_or(Engine::Skate3.default_scale(), Engine::default_scale);
    let id = {
        let mut parks = world.resource_mut::<Parks>();
        let id = new_id(&parks.list);
        let name = next_name(&parks.list, course_name(mode));
        let at = now();
        parks.list.push(ParkEntry { id: id.clone(), file: ParkFile { version: VERSION, name, created: at, opened: at, pieces: Vec::new(), scale, park_scale: 1.0, anchors: Vec::new(), engine: None, engine_settings: None, mode: mode.key().to_string() }, edits: 0 });
        id
    };
    let file = world.resource::<Parks>().get(&id).expect("just made").file.clone();
    open_as(world, &id, &file, ParkPose::default(), scale, None);
    id
}

/// Course `id` named `name` (trimmed; an empty name leaves it as it was).
pub(crate) fn rename(world: &mut World, id: &str, name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return false;
    }
    let mut parks = world.resource_mut::<Parks>();
    let Some(entry) = parks.get_mut(id) else { return false };
    entry.file.name = name.to_string();
    parks.save(id);
    true
}

/// Course `id` made a course of `mode` (it moves to that mode's tab): the engine it was played with is forgotten, and
/// when it is the open one, that mode's engine runs it.
pub(crate) fn set_mode(world: &mut World, id: &str, mode: Mode) {
    {
        let mut parks = world.resource_mut::<Parks>();
        let Some(entry) = parks.get_mut(id) else { return };
        if entry.file.mode() == Some(mode) {
            return;
        }
        entry.file.mode = mode.key().to_string();
        entry.file.engine = None;
        entry.file.engine_settings = None;
        parks.save(id);
    }
    if world.resource::<Parks>().is_current(id)
        && let Some(engine) = engine_of_mode(world, mode)
        && world.get_resource::<Engines>().and_then(|e| e.chosen()) != Some(engine)
    {
        world.resource_mut::<Engines>().request(engine);
    }
}

/// A copy of park `id`, in the same places.
pub(crate) fn duplicate(world: &mut World, id: &str) -> Option<String> {
    let mut parks = world.resource_mut::<Parks>();
    let mut file = parks.get(id)?.file.clone();
    let copy = new_id(&parks.list);
    file.name = next_name(&parks.list, &format!("{} copy", file.name));
    file.created = now();
    file.opened = file.created;
    parks.list.push(ParkEntry { id: copy.clone(), file, edits: 0 });
    parks.save(&copy);
    Some(copy)
}

/// Park `id` gone, and the anchors of it that the headset has found erased.
pub(crate) fn delete(world: &mut World, id: &str) {
    let Some(entry) = world.resource::<Parks>().get(id).cloned() else { return };
    let shared: Vec<String> = world.resource::<Parks>().list.iter().filter(|p| p.id != id).flat_map(|p| p.file.anchors.iter().map(|r| r.uuid.clone())).collect();
    {
        let mut anchors = world.resource_mut::<Anchors>();
        for record in entry.file.anchors.iter().filter(|r| !shared.contains(&r.uuid)) {
            anchors.erase(&record.uuid);
        }
    }
    if world.resource::<Parks>().is_current(id) {
        world.resource_mut::<Parks>().current = None;
        world.resource_mut::<PlacedPieces>().replace(Vec::new());
    }
    let mut parks = world.resource_mut::<Parks>();
    if let Some(dir) = parks.dir.clone() {
        if let Err(e) = std::fs::remove_file(file_path(&dir, id)) {
            warn!("PARKS could not delete {id}: {e}");
        }
    }
    parks.list.retain(|p| p.id != id);
    parks.revision += 1;
    info!("PARKS deleted {id}");
}

/// Park `id` no longer kept in this room: the anchors of it found here are dropped.
pub(crate) fn forget_here(world: &mut World, id: &str) {
    let here: Vec<String> = {
        let anchors = world.resource::<Anchors>();
        world.resource::<Parks>().get(id).map(|p| p.file.anchors.iter().filter(|r| anchors.located(&r.uuid).is_some()).map(|r| r.uuid.clone()).collect()).unwrap_or_default()
    };
    let shared: Vec<String> = world.resource::<Parks>().list.iter().filter(|p| p.id != id).flat_map(|p| p.file.anchors.iter().map(|r| r.uuid.clone())).collect();
    for uuid in &here {
        if !shared.contains(uuid) {
            world.resource_mut::<Anchors>().erase(uuid);
        }
    }
    let mut parks = world.resource_mut::<Parks>();
    if let Some(entry) = parks.get_mut(id) {
        entry.file.anchors.retain(|r| !here.contains(&r.uuid));
    }
    parks.save(id);
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::xr::build::pieces::{Piece, PieceKind};

    fn piece(id: u64, position: [f32; 3], yaw: f32) -> Placed {
        Placed { id, piece: Piece::new(PieceKind::Kicker), position, yaw, scale: 1.0, hidden: false, no_posts: 0, ends_down: [0.0; 2], finish: None, tint: None, path: None }
    }

    fn near(a: [f32; 3], b: Vec3) -> bool {
        Vec3::from_array(a).distance(b) < 1e-4
    }

    #[test]
    fn a_closed_course_resized_and_back_is_as_it_was_and_the_same_real_size() {
        use crate::xr::build::size::{MAX_TIMES, MIN_TIMES, resize_file, times};
        let mut file = ParkFile {
            version: VERSION,
            name: "Track".into(),
            created: 1,
            opened: 1,
            pieces: vec![piece(1, [8.0, 0.0, -16.0], 0.0)],
            scale: 8.0,
            park_scale: 0.4,
            anchors: vec![AnchorRecord { uuid: "a".into(), offset: [0.0; 3], turn: 0.0, scale: 8.0, placed: 0 }],
            engine: None,
            engine_settings: None,
            mode: "race".into(),
        };
        // (each piece's real place and size: game metres over the scale)
        let real = |f: &ParkFile| (Vec3::from_array(f.pieces[0].position) / f.scale, f.park_scale / f.scale);
        let before = real(&file);
        for t in [MAX_TIMES, MIN_TIMES, 2.5] {
            resize_file(&mut file, t, 8.0);
            assert!((times(file.scale, 8.0) - t).abs() < 1e-4);
            assert!((real(&file).0 - before.0).length() < 1e-4 && (real(&file).1 - before.1).abs() < 1e-6, "the same real size at x{t}");
            assert!((file.anchors[0].scale - file.scale).abs() < 1e-3, "its room's record follows");
        }
        resize_file(&mut file, 1.0, 8.0);
        assert!((file.park_scale - 0.4).abs() < 1e-5 && near(file.pieces[0].position, Vec3::new(8.0, 0.0, -16.0)), "back as it was");
    }

    #[test]
    fn a_park_goes_into_the_room_and_back_unchanged() {
        let pieces = vec![piece(1, [0.0, 0.0, 0.0], 0.0), piece(2, [8.0, 2.0, -4.0], 0.5)];
        let pose = ParkPose { at: Vec3::new(1.0, 0.7, -2.0), yaw: 1.2 };
        let room = in_room(&pieces, pose, 8.0);
        // A metre of the room is 8 game metres: the second piece is 1 m along the frame's x, 25 cm up, 50 cm along its z.
        assert!(near(room[1].position, pose.at + Quat::from_rotation_y(1.2) * Vec3::new(1.0, 0.25, -0.5)));
        assert!((room[1].yaw - 1.7).abs() < 1e-6);
        let back = in_park(&room, pose, 8.0);
        for (a, b) in back.iter().zip(&pieces) {
            assert!(near(a.position, Vec3::from_array(b.position)) && (a.yaw - b.yaw).abs() < 1e-5);
        }
        // At twice the world scale (a skater half the size) the same park is half as big in the room.
        let small = in_room(&pieces, pose, 16.0);
        assert!((Vec3::from_array(small[1].position).distance(pose.at) * 2.0 - Vec3::from_array(room[1].position).distance(pose.at)).abs() < 1e-4);
    }

    #[test]
    fn an_anchor_keeps_the_park_where_it_was_put() {
        let pose = ParkPose { at: Vec3::new(2.0, 0.75, -1.0), yaw: 0.4 };
        let (anchor, turn) = (Vec3::new(1.5, 0.75, -1.2), -0.3);
        let record = pose.record("u".into(), anchor, turn, 8.0, 0);
        let back = ParkPose::by_anchor(anchor, turn, &record);
        assert!(back.at.distance(pose.at) < 1e-5 && (back.yaw - pose.yaw).abs() < 1e-6);
        // The tracking space moved and turned: the park moves with its anchor.
        let (moved_to, moved_turn) = (Vec3::new(-0.5, 0.72, 0.3), 1.1);
        let then = ParkPose::by_anchor(moved_to, moved_turn, &record);
        assert!((then.at.distance(moved_to) - pose.at.distance(anchor)).abs() < 1e-5);
        assert!((then.yaw - (pose.yaw + moved_turn - turn)).abs() < 1e-6);
        let mut items = in_room(&[piece(1, [4.0, 0.0, 0.0], 0.0)], pose, 8.0);
        moved(&mut items, pose, then);
        assert!(near(items[0].position, then.to_room(Vec3::new(4.0, 0.0, 0.0), 8.0)));
    }

    #[test]
    fn a_park_is_held_under_its_middle_and_put_where_the_pointer_is() {
        let pieces = vec![piece(1, [-4.0, 0.0, 2.0], 0.0), piece(2, [6.0, 1.0, -2.0], 0.0)];
        assert_eq!(pivot(&pieces), Vec3::new(1.0, 0.0, 0.0));
        let job = PlaceJob { id: "p".into(), edit: false, pivot: pivot(&pieces), pieces: pieces.clone(), yaw: 0.7, scale: 4.0, default: 12.0, at: None, before: None, turned: false };
        let at = Vec3::new(0.3, 0.74, -1.5);
        let pose = job.pose(at);
        // Its middle, at its lowest, is where the pointer is.
        assert!(pose.to_room(job.pivot, 4.0).distance(at) < 1e-5);
        assert_eq!(pivot(&[]), Vec3::ZERO);
    }

    #[test]
    fn the_old_layout_becomes_a_park_that_opens_where_it_was() {
        // Pieces in the tracking space, the anchor where it was found when they were written.
        let old: OldLayout = serde_json::from_str(
            r#"{"version":1,"scale":8.0,"park_scale":1.5,"anchor":{"uuid":"000102030405060708090a0b0c0d0e0f","position":[0.5,0.0,-1.0],"yaw":0.25},
                "pieces":[{"id":1,"piece":{"kind":"Rail","size":1.0},"position":[1.0,0.75,-1.0],"yaw":0.5}]}"#,
        )
        .unwrap();
        let file = from_old(old, 10.0, 7).unwrap();
        assert_eq!((file.scale, file.park_scale, file.anchors.len(), file.created), (8.0, 1.5, 1, 7));
        // Found where it was: the piece is where it was.
        let pose = ParkPose::by_anchor(Vec3::new(0.5, 0.0, -1.0), 0.25, &file.anchors[0]);
        let room = in_room(&file.pieces, pose, file.anchors[0].scale);
        assert!(near(room[0].position, Vec3::new(1.0, 0.75, -1.0)) && (room[0].yaw - 0.5).abs() < 1e-5, "{:?}", room[0]);
        // A layout without a scale takes the one it is given; one from another version is left.
        let bare: OldLayout = serde_json::from_str(r#"{"version":1,"pieces":[]}"#).unwrap();
        assert_eq!(from_old(bare, 10.0, 0).unwrap().scale, 10.0);
        assert!(from_old(serde_json::from_str(r#"{"version":5,"pieces":[]}"#).unwrap(), 10.0, 0).is_none());
    }

    #[test]
    fn parks_are_written_and_read_back_and_named_apart() {
        let dir = std::env::temp_dir().join(format!("sk8-parks-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = ParkFile {
            version: VERSION,
            name: "Desk lines".into(),
            created: 1,
            opened: 2,
            pieces: vec![piece(1, [1.0, 2.0, 3.0], 0.5)],
            scale: 8.0,
            park_scale: 1.25,
            anchors: vec![AnchorRecord { uuid: "ab".repeat(16), offset: [0.1, 0.0, -0.2], turn: 0.3, scale: 8.0, placed: 9 }],
            engine: None,
            engine_settings: None,
            mode: skate_mode(),
        };
        write(&dir, "a1", &file).unwrap();
        std::fs::write(dir.join("junk.json"), b"{").unwrap();
        let mut track = serde_json::to_value(&file).unwrap();
        track["mode"] = "race".into();
        std::fs::write(dir.join("t1.json"), serde_json::to_vec(&track).unwrap()).unwrap();
        let mut unknown = serde_json::to_value(&file).unwrap();
        unknown["mode"] = "fly".into();
        std::fs::write(dir.join("u1.json"), serde_json::to_vec(&unknown).unwrap()).unwrap();
        let mut old = serde_json::to_value(&file).unwrap();
        old.as_object_mut().unwrap().remove("mode");
        std::fs::write(dir.join("o1.json"), serde_json::to_vec(&old).unwrap()).unwrap();
        let mut read = read_all(&dir);
        read.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(read.len(), 3, "the unreadable one and the course of a mode this app doesn't know are left out");
        assert_eq!(read[0], ("a1".to_string(), file.clone()));
        assert_eq!(read[1], ("o1".to_string(), file.clone()), "a park from before modes is a skate park");
        assert_eq!((read[2].0.as_str(), read[2].1.mode()), ("t1", Some(Mode::Race)), "a race track");
        assert!(serde_json::to_string(&file).unwrap().contains(r#""mode":"skate""#));
        let _ = std::fs::remove_dir_all(&dir);
        let entry = |name: &str| ParkEntry { id: name.into(), file: ParkFile { name: name.into(), ..file.clone() }, edits: 0 };
        assert_eq!(next_name(&[entry("Park 1")], "Park"), "Park 2");
        assert_eq!(next_name(&[], "Park"), "Park 1");
        assert_eq!(next_name(&[entry("Desk copy")], "Desk copy"), "Desk copy 2");
        assert_eq!((course_name(Mode::Race), courses_name(Mode::Race), course_name(Mode::Skate)), ("Track", "Tracks", "Park"));
    }

    /// The open park keeps the engine it is played with and that engine's park settings, written to its file; a park
    /// whose own engine isn't installed keeps its tag while it is played with another; opened, a park's settings come
    /// back.
    #[test]
    fn the_open_park_keeps_its_engine_and_settings() {
        use crate::engines::{self, Engine};
        #[derive(Resource)]
        struct Size(f32);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).insert_resource(Size(2.0)).init_resource::<CourseSettings>().init_resource::<Placing>();
        app.world_mut().resource_mut::<CourseSettings>().0.insert(
            Engine::Thug2,
            (|w: &World| serde_json::json!({ "size": w.resource::<Size>().0 }), |w: &mut World, v: &serde_json::Value| w.resource_mut::<Size>().0 = v["size"].as_f64().unwrap() as f32),
        );
        engines::add_engine(&mut app, Engine::Thug2, |_| Ok(()), |app| {
            app.add_systems(Update, || {});
        });
        engines::add_engine(&mut app, Engine::Skate3, |_| Err("not imported".into()), |app| {
            app.add_systems(Update, || {});
        });
        app.update();
        let dir = std::env::temp_dir().join(format!("sk8-parks-engine-test-{}", std::process::id()));
        let file = |engine: Option<&str>| ParkFile {
            version: VERSION,
            name: "p".into(),
            created: 0,
            opened: 0,
            pieces: Vec::new(),
            scale: 8.0,
            park_scale: 1.0,
            anchors: Vec::new(),
            engine: engine.map(str::to_string),
            engine_settings: None,
            mode: skate_mode(),
        };
        let mut parks = Parks { dir: Some(dir.clone()), ..default() };
        parks.list.push(ParkEntry { id: "old".into(), file: file(None), edits: 0 });
        parks.list.push(ParkEntry { id: "s3".into(), file: file(Some("skate3")), edits: 0 });
        parks.current = Some(Current { id: "old".into(), pose: ParkPose::default(), anchor: None });
        app.insert_resource(parks);
        tag_engine(app.world_mut());
        let kept = |app: &App, id: &str| app.world().resource::<Parks>().get(id).unwrap().file.clone();
        assert_eq!(kept(&app, "old").engine.as_deref(), Some("thug2"));
        assert_eq!(kept(&app, "old").engine_settings, Some(serde_json::json!({ "size": 2.0 })));
        assert_eq!(read_all(&dir).into_iter().find(|(id, _)| id == "old").unwrap().1, kept(&app, "old"), "written");
        // Its setting changed: kept too.
        app.world_mut().resource_mut::<Size>().0 = 3.0;
        tag_engine(app.world_mut());
        assert_eq!(kept(&app, "old").engine_settings, Some(serde_json::json!({ "size": 3.0 })));
        // Opened again elsewhere at another size: its own comes back.
        app.world_mut().resource_mut::<Size>().0 = 1.0;
        use_its_engine(app.world_mut(), "old");
        assert_eq!(app.world().resource::<Size>().0, 3.0);
        // Skate 3's park, played with THUG2 (Skate 3 isn't installed): still Skate 3's.
        app.world_mut().resource_mut::<Parks>().current = Some(Current { id: "s3".into(), pose: ParkPose::default(), anchor: None });
        tag_engine(app.world_mut());
        assert_eq!(kept(&app, "s3").engine.as_deref(), Some("skate3"));
        assert_eq!(kept(&app, "s3").engine_settings, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A course's engine settings stepped in its settings dialog: its default until it keeps one; set, kept with
    /// it (and written), and in play only if it is the open course.
    #[test]
    fn a_courses_stepped_settings_are_kept_with_it() {
        use crate::engines::Engine;
        #[derive(Resource, Default)]
        struct Live(Option<serde_json::Value>);
        let mut world = World::new();
        world.init_resource::<Live>();
        world.init_resource::<CourseSettings>();
        world.init_resource::<CourseSteppers>();
        world.resource_mut::<CourseSettings>().0.insert(
            Engine::Mkwii,
            (|w: &World| w.resource::<Live>().0.clone().unwrap_or_default(), |w: &mut World, v: &serde_json::Value| w.resource_mut::<Live>().0 = Some(v.clone())),
        );
        let speed = CourseStepper { key: "speed", label: "Top speed", help: "", range: (0.5, 2.0, 0.05), default: 1.0, percent: true };
        world.resource_mut::<CourseSteppers>().0.insert(Engine::Mkwii, vec![speed]);
        let dir = std::env::temp_dir().join(format!("sk8-parks-steppers-test-{}", std::process::id()));
        let track = ParkFile {
            version: VERSION,
            name: "t".into(),
            created: 0,
            opened: 0,
            pieces: Vec::new(),
            scale: 8.0,
            park_scale: 1.0,
            anchors: Vec::new(),
            engine: Some("mkwii".into()),
            engine_settings: None,
            mode: "race".into(),
        };
        let mut parks = Parks { dir: Some(dir.clone()), ..default() };
        parks.list.push(ParkEntry { id: "a".into(), file: track.clone(), edits: 0 });
        parks.list.push(ParkEntry { id: "b".into(), file: track, edits: 0 });
        parks.current = Some(Current { id: "a".into(), pose: ParkPose::default(), anchor: None });
        world.insert_resource(parks);
        assert_eq!(course_steppers(&world, "b"), vec![speed]);
        assert_eq!(stepper_value(&world, "b", &speed), 1.0, "its default");
        // (a course not open: kept, not played)
        set_stepper(&mut world, "b", &speed, speed.stepped(1.0, 2));
        assert!((stepper_value(&world, "b", &speed) - 1.1).abs() < 1e-5);
        assert_eq!(world.resource::<Live>().0, None);
        let written = read_all(&dir).into_iter().find(|(id, _)| id == "b").unwrap().1;
        assert!((written.engine_settings.unwrap()["speed"].as_f64().unwrap() - 1.1).abs() < 1e-5, "written");
        // (the open course: in play too)
        set_stepper(&mut world, "a", &speed, 1.5);
        assert_eq!(world.resource::<Live>().0, Some(serde_json::json!({ "speed": 1.5 })));
        assert_eq!(stepper_value(&world, "a", &speed), 1.5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parks_here_come_first_then_the_last_opened() {
        let file = |name: &str, opened: u64, uuid: &str| ParkFile {
            version: VERSION,
            name: name.into(),
            created: 0,
            opened,
            pieces: Vec::new(),
            scale: 8.0,
            park_scale: 1.0,
            anchors: vec![AnchorRecord { uuid: uuid.into(), offset: [0.0; 3], turn: 0.0, scale: 8.0, placed: 0 }],
            engine: None,
            engine_settings: None,
            mode: skate_mode(),
        };
        let mut parks = Parks::default();
        for (id, f) in [("a", file("Old here", 1, "u1")), ("b", file("New away", 9, "u2")), ("c", file("New here", 5, "u3")), ("d", file("Old away", 2, "u4"))] {
            parks.list.push(ParkEntry { id: id.into(), file: f, edits: 0 });
        }
        let mut anchors = Anchors::default();
        assert!(parks.sorted(&anchors).iter().all(|(_, h)| *h == Here::Looking) == anchors.searching());
        anchors.found_for_test(&["u1", "u3"]);
        let order: Vec<(&str, Here)> = parks.sorted(&anchors).into_iter().map(|(i, h)| (parks.list[i].id.as_str(), h)).collect();
        assert_eq!(order, vec![("c", Here::Found), ("a", Here::Found), ("b", Here::Away), ("d", Here::Away)]);
    }
}
