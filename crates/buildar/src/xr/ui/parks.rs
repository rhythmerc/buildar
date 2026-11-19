//! The parks screen: what the menu opens on (at launch, and on a menu tap
//! while skating). A tab for each mode's courses (Parks for skating, Tracks for
//! racing), and on it a card for each saved course ([`crate::xr::build::parks`])
//! with a play and an edit button, and a card to start a new one first.
//! Parks found in this room come first; past a divider, the ones that have to
//! be put down by hand. Each lot by when it was last opened. The open park is
//! ringed. Settings (the old start page) is the gear in the top right corner.
//!
//! Worked with the laser (the trigger on a card plays it; on its pencil,
//! builds it) or the sticks (move, A presses, the right grip opens a park's
//! "more" menu, B goes back to skating).
mod course;

use super::{pages, Nav, Outcome, XrUi, UI_BODY_FONT, UI_FONT};
use super::pointer::UiPointer;
use crate::xr::build::anchor::Anchors;
use crate::engines::{Engine, EngineToggles, Engines, Mode};
use crate::xr::build::parks::{self as park, EngineFit, Here, ParkFile, Parks};
use crate::xr::build::{BuildMode, size};
use crate::xr::panel::XrPanel;
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::HashMap;

const COLUMNS: usize = 3;
/// The cards' room on the panel, px, and each line's height (a card's, the
/// divider's), each with the gap under it.
const BUDGET_PX: f32 = 693.0;
const GAP_PX: f32 = 22.0;
const CARD_W: f32 = 292.0;
const CARD_H: f32 = 301.0;
const THUMB_H: f32 = 182.0;
const DIVIDER_PX: f32 = 20.0;
const THUMB: (u32, u32) = (320, 200);
const ICON: u32 = 64;
/// A name longer than this many letters ends in "…".
const NAME_LETTERS: usize = 19;

// The panel's palette (the wireframe's, and the menus' teal).
const PANEL: Color = Color::srgba(0.055, 0.082, 0.09, 0.97);
const CARD: Color = Color::srgb(0.09, 0.133, 0.149);
const THUMB_BG: Color = Color::srgb(0.114, 0.165, 0.184);
const LINE: Color = Color::srgb(0.169, 0.231, 0.251);
const TEAL: Color = Color::srgb(0.451, 0.902, 0.851);
const SELECTED: Color = Color::srgb(0.122, 0.42, 0.4);
const DIM: Color = Color::srgb(0.722, 0.78, 0.8);
const OFF: Color = Color::srgb(0.549, 0.58, 0.6);
const INK: Color = Color::srgb(0.055, 0.082, 0.09);
const DANGER: Color = Color::srgb(0.941, 0.541, 0.494);
const MENU: Color = Color::srgb(0.039, 0.063, 0.071);
const FOCUS: Color = Color::WHITE;

/// What the laser can press.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    Settings,
    /// A mode's tab ([`TABS`]).
    Tab(usize),
    /// The engine dropdown's button, and its rows.
    Engine,
    EngineRow(usize),
    /// The missing engine's warning: "Play with current engine" (true) or "Cancel".
    Warn(bool),
    /// A card (the new-park card, or a park's: it plays it).
    Item(usize),
    Edit(usize),
    Play(usize),
    More(usize),
    MenuRow(usize),
    Confirm(bool),
    /// The course settings: its size a step bigger (true) or smaller; its engine's setting (by its place) likewise;
    /// and Done.
    SizeStep(bool),
    EngineStep(usize, bool),
    /// The small gear by an engine's dropdown (its engine settings), and in that dialog a toggle (by its place).
    EngineGear,
    Toggle(usize),
    SettingsDone,
    /// Around an open menu: closes it.
    Backdrop,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Part {
    /// The card itself (play).
    #[default]
    Main,
    Edit,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Focus {
    Tab(usize),
    Settings,
    Engine,
    EngineGear,
    Item(usize, Part),
}

impl Default for Focus {
    fn default() -> Self {
        Focus::Item(0, Part::Main)
    }
}

#[derive(Clone, PartialEq, Debug)]
struct CardView {
    id: String,
    name: String,
    sub: String,
    here: Here,
    current: bool,
    thumb: Option<Handle<Image>>,
    /// It has no pieces yet.
    empty: bool,
    /// The engine it plays on (its own, whichever engine is picked); none: it plays on the one picked.
    engine: Option<&'static str>,
}

#[derive(Clone, PartialEq, Debug)]
enum Item {
    /// A new course of the tab's mode.
    New(Mode),
    Park(CardView),
}

/// The tabs: a mode's courses each.
const TABS: [Mode; 2] = [Mode::Skate, Mode::Race];

#[derive(Clone, PartialEq, Debug)]
enum Line {
    Cards(Vec<usize>),
    Divider(&'static str),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MenuAction {
    Settings,
    Duplicate,
    PlaceElsewhere,
    Forget,
    Delete,
}

impl MenuAction {
    fn label(self) -> &'static str {
        match self {
            Self::Settings => "Course settings",
            Self::Duplicate => "Duplicate",
            Self::PlaceElsewhere => "Put it somewhere else",
            Self::Forget => "Forget it in this room",
            Self::Delete => "Delete park",
        }
    }
}

/// What the screen shows (drawn again whenever it changes).
#[derive(Clone, PartialEq, Debug, Default)]
struct Model {
    /// The tab shown ([`TABS`]).
    tab: usize,
    items: Vec<Item>,
    lines: Vec<Line>,
    first: usize,
    focus: Option<Focus>,
    /// A park's "more" menu: its name, the rows, the one chosen.
    menu: Option<(String, Vec<MenuAction>, usize)>,
    /// A park about to be deleted: its name, and which button is chosen.
    confirm: Option<(String, bool)>,
    /// A course's settings open.
    settings: Option<SettingsView>,
    /// An engine's settings open.
    engine_settings: Option<EngineSettingsView>,
    /// The engine in the dropdown has settings of the player's (its gear shown).
    engine_gear: bool,
    /// A course's details, in place of the cards.
    details: Option<course::View>,
    /// The engine running, and (open) the dropdown's rows and the one chosen.
    engine: Option<(String, Option<(Vec<EngineRow>, usize)>)>,
    /// A park built for an engine that isn't installed: that engine's name, and whether "Play with current engine"
    /// is chosen.
    warning: Option<(String, bool)>,
    status: String,
}

/// A course's settings as shown: its title, its name, its size and whether it can go smaller and
/// bigger, its engine's settings the player steps, and the row chosen.
#[derive(Clone, PartialEq, Debug)]
struct SettingsView {
    title: String,
    name: String,
    size: String,
    smaller: bool,
    bigger: bool,
    steppers: Vec<StepperView>,
    row: SettingsRow,
}

/// An engine's setting of the course as shown (`engines::CourseStepper`): its label, its line, its value, and
/// whether it can go down and up.
#[derive(Clone, PartialEq, Debug)]
struct StepperView {
    label: String,
    help: String,
    value: String,
    down: bool,
    up: bool,
}

/// An engine's settings as shown (the player's own, `engines::EngineToggles`): its name, each toggle's label, line
/// and value, and the row chosen (the toggles', then Done).
#[derive(Clone, PartialEq, Debug)]
pub(super) struct EngineSettingsView {
    name: String,
    toggles: Vec<(String, String, bool)>,
    row: usize,
}

/// The dialog's rows: the player scale, the engine's settings (each by its place), Done.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum SettingsRow {
    #[default]
    Size,
    Engine(usize),
    Done,
}

impl SettingsRow {
    /// The row above (`up`) or below, of a dialog with `engine` engine settings (round from the ends).
    fn moved(self, up: bool, engine: usize) -> Self {
        let rows: Vec<SettingsRow> =
            std::iter::once(SettingsRow::Size).chain((0..engine).map(SettingsRow::Engine)).chain(std::iter::once(SettingsRow::Done)).collect();
        let at = rows.iter().position(|r| *r == self).unwrap_or(0);
        rows[if up { (at + rows.len() - 1) % rows.len() } else { (at + 1) % rows.len() }]
    }
}

/// An engine in the dropdown.
#[derive(Clone, PartialEq, Debug)]
struct EngineRow {
    engine: Engine,
    name: String,
    /// It can be switched to (else greyed out, `why` says why not).
    installed: bool,
    why: String,
    running: bool,
}

/// What opening a park was going to do, once its engine is settled.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Then {
    /// Play it (or, `true`, build it).
    Open(bool),
    /// Put it down somewhere else.
    Place,
}

#[derive(Resource, Default)]
pub(crate) struct ParksScreen {
    /// The tab shown ([`TABS`]).
    tab: usize,
    focus: Focus,
    first: usize,
    /// The park whose menu is open, and the row chosen.
    menu: Option<(String, usize)>,
    /// The park asked to be deleted, and whether "Delete" is chosen.
    confirm: Option<(String, bool)>,
    /// A course's details open (the open course's, the menu opened while playing it): in place of the parks.
    details: Option<course::Details>,
    /// The course whose settings are open, the row chosen, and how long the trigger has been held on a stepper
    /// button (it repeats, as a held stick does).
    settings: Option<(String, SettingsRow)>,
    /// An engine's settings open (the gear by its dropdown), and the row chosen (its toggles', then Done).
    pub(super) engine_settings: Option<(Engine, usize)>,
    step_held: f32,
    /// The engine dropdown is open: the row chosen.
    engines: Option<usize>,
    /// A park whose engine isn't installed, what opening it was going to do, and whether "Play with current engine"
    /// is chosen.
    warning: Option<(String, Then, bool)>,
    model: Model,
    drawn: Option<Model>,
    thumbs: HashMap<String, (u64, Handle<Image>)>,
    icons: Option<Icons>,
    fonts: Option<Fonts>,
    root: Option<Entity>,
}

#[derive(Clone)]
struct Icons {
    play: Handle<Image>,
    pencil: Handle<Image>,
    more: Handle<Image>,
    gear: Handle<Image>,
    back: Handle<Image>,
    trash: Handle<Image>,
    chevron: Handle<Image>,
    sliders: Handle<Image>,
    check: Handle<Image>,
}

/// The display face (titles, names) and the running text's.
#[derive(Clone)]
struct Fonts {
    display: Handle<Font>,
    body: Handle<Font>,
}

impl Fonts {
    fn display(&self, size: f32, weight: u16) -> TextFont {
        TextFont { font: self.display.clone(), font_size: size, weight: FontWeight(weight), ..default() }
    }

    fn body(&self, size: f32, weight: u16) -> TextFont {
        TextFont { font: self.body.clone(), font_size: size, weight: FontWeight(weight), ..default() }
    }
}

#[derive(Component)]
struct ParksRoot;

// ---------------------------------------------------------------- logic

fn when(opened: u64, now: u64) -> String {
    let days = now.saturating_sub(opened) / 86_400;
    match days {
        0 => "today".into(),
        1 => "yesterday".into(),
        2..=13 => format!("{days} days ago"),
        14..=59 => format!("{} weeks ago", days / 7),
        _ => format!("{} months ago", days / 30),
    }
}

fn subtitle(file: &ParkFile, current: bool, now: u64) -> String {
    let pieces = match file.pieces.len() {
        0 => "empty".to_string(),
        1 => "1 piece".to_string(),
        n => format!("{n} pieces"),
    };
    // (two lines beside the buttons)
    format!("{pieces}\n{}", if current { "open now".to_string() } else { when(file.opened, now) })
}

/// The cards in lines: the new-park card and the parks here, then a divider
/// and the rest.
fn lines(items: &[Item]) -> Vec<Line> {
    let away = |item: &Item| matches!(item, Item::Park(c) if c.here != Here::Found);
    let split = items.iter().position(away).unwrap_or(items.len());
    let mut out: Vec<Line> = (0..split).collect::<Vec<_>>().chunks(COLUMNS).map(|c| Line::Cards(c.to_vec())).collect();
    if split < items.len() {
        let looking = items[split..].iter().any(|i| matches!(i, Item::Park(c) if c.here == Here::Looking));
        out.push(Line::Divider(if looking { "Looking in this room…" } else { "Not found in this room" }));
        out.extend((split..items.len()).collect::<Vec<_>>().chunks(COLUMNS).map(|c| Line::Cards(c.to_vec())));
    }
    out
}

fn line_of(lines: &[Line], item: usize) -> Option<(usize, usize)> {
    lines.iter().enumerate().find_map(|(l, line)| match line {
        Line::Cards(c) => c.iter().position(|&i| i == item).map(|col| (l, col)),
        Line::Divider(_) => None,
    })
}

fn height(line: &Line) -> f32 {
    GAP_PX
        + match line {
            Line::Cards(_) => CARD_H,
            Line::Divider(_) => DIVIDER_PX,
        }
}

/// The first line shown, so that line `focus` is on screen.
fn scrolled(lines: &[Line], first: usize, focus: usize) -> usize {
    let mut first = first.min(focus).min(lines.len().saturating_sub(1));
    while first < focus && lines[first..=focus].iter().map(height).sum::<f32>() > BUDGET_PX {
        first += 1;
    }
    // (a divider is never left alone at the top over nothing)
    if first > 0 && matches!(lines.get(first - 1), Some(Line::Divider(_))) && lines[first - 1..=focus].iter().map(height).sum::<f32>() <= BUDGET_PX {
        first -= 1;
    }
    first
}

/// The lines shown from `first`.
fn shown(lines: &[Line], first: usize) -> usize {
    let mut used = 0.0;
    lines[first.min(lines.len())..]
        .iter()
        .take_while(|l| {
            used += height(l);
            used <= BUDGET_PX
        })
        .count()
}

/// The stick moves the focus between cards, and on a park's card between
/// its pencil and itself.
fn moved(focus: Focus, nav: &Nav, items: &[Item], lines: &[Line]) -> Focus {
    let parts = |i: usize| matches!(items.get(i), Some(Item::Park(_)));
    let last_tab = TABS.len() - 1;
    match focus {
        Focus::Tab(t) if nav.left && t > 0 => Focus::Tab(t - 1),
        Focus::Tab(t) if nav.right && t < last_tab => Focus::Tab(t + 1),
        Focus::Tab(_) if nav.right => Focus::Engine,
        Focus::Tab(_) if nav.down => Focus::Item(0, Part::Main),
        Focus::Tab(_) => focus,
        // (the buildAR Menu, bottom right: up to the last cards)
        Focus::Settings if nav.up => Focus::Item(items.len() - 1, Part::Main),
        Focus::Settings => focus,
        Focus::Engine if nav.left => Focus::Tab(last_tab),
        Focus::Engine if nav.right => Focus::EngineGear,
        Focus::EngineGear if nav.left => Focus::Engine,
        Focus::EngineGear if nav.down => Focus::Item(0, Part::Main),
        Focus::EngineGear => focus,
        Focus::Engine if nav.down => Focus::Item(0, Part::Main),
        Focus::Engine => focus,
        Focus::Item(i, part) => {
            if nav.right {
                return match part {
                    Part::Edit => Focus::Item(i, Part::Main),
                    Part::Main if i + 1 < items.len() => Focus::Item(i + 1, if parts(i + 1) { Part::Edit } else { Part::Main }),
                    Part::Main => focus,
                };
            }
            if nav.left {
                return match part {
                    Part::Main if parts(i) => Focus::Item(i, Part::Edit),
                    _ if i > 0 => Focus::Item(i - 1, Part::Main),
                    _ => focus,
                };
            }
            if !(nav.up || nav.down) {
                return focus;
            }
            let Some((line, col)) = line_of(lines, i) else { return focus };
            let step: isize = if nav.up { -1 } else { 1 };
            let mut l = line as isize + step;
            while let Some(next) = usize::try_from(l).ok().and_then(|l| lines.get(l)) {
                if let Line::Cards(c) = next {
                    let j = c[col.min(c.len() - 1)];
                    return Focus::Item(j, if parts(j) { part } else { Part::Main });
                }
                l += step;
            }
            if nav.up { Focus::Tab(0) } else { Focus::Settings }
        }
    }
}

/// The menu opened (the menu button): on the open course's details while a course is open, else on the parks.
pub(crate) fn opened_by_menu(world: &mut World) {
    let current = world.resource::<Parks>().current.as_ref().map(|c| c.id.clone());
    let mut screen = world.resource_mut::<ParksScreen>();
    screen.details = current.map(course::Details::new);
    screen.menu = None;
    screen.settings = None;
    screen.engine_settings = None;
    screen.confirm = None;
}

/// The menu opened some other way (at launch, a page asked for): on the parks.
pub(crate) fn opened_on_parks(world: &mut World) {
    world.resource_mut::<ParksScreen>().details = None;
}

fn menu_for(here: Here, current: bool) -> Vec<MenuAction> {
    let mut rows = vec![MenuAction::Settings, MenuAction::Duplicate, MenuAction::PlaceElsewhere];
    // (the open park is kept here: forgetting it would only anchor it again)
    if here == Here::Found && !current {
        rows.push(MenuAction::Forget);
    }
    rows.push(MenuAction::Delete);
    rows
}

/// The engines for the dropdown, those the menus offer for courses of `mode` (none: the app has no engine choice
/// for them).
fn engine_rows(world: &World, mode: Mode) -> Option<Vec<EngineRow>> {
    let engines = world.get_resource::<Engines>()?;
    let rows: Vec<EngineRow> = Engine::ALL
            .iter()
            .filter(|&&e| e.mode() == mode && engines.listed(e, world))
            .map(|&e| {
                let why = engines.available(e, world).err().unwrap_or_default();
                EngineRow { engine: e, name: e.name().to_string(), installed: why.is_empty(), why, running: engines.chosen() == Some(e) }
            })
            .collect();
    (!rows.is_empty()).then_some(rows)
}

/// Does `then` with park `id`, its engine first: switched to when it's installed, else the warning that it isn't.
fn open_with_engine(world: &mut World, id: &str, then: Then) -> Outcome {
    if let EngineFit::Missing(_) = park::engine_fit(world, id) {
        world.resource_mut::<ParksScreen>().warning = Some((id.to_string(), then, false));
        return Outcome::Stay;
    }
    park::use_its_engine(world, id);
    carry_out(world, id, then)
}

/// A park's Play pressed (`xr::dev_play`: unattended, on the simulator).
pub(super) fn play(world: &mut World, id: &str) -> Outcome {
    open_with_engine(world, id, Then::Open(false))
}

fn carry_out(world: &mut World, id: &str, then: Then) -> Outcome {
    match then {
        Then::Open(edit) => open(world, id, edit),
        Then::Place if park::start_placing(world, id, false) => Outcome::Close,
        Then::Place => Outcome::Stay,
    }
}

/// Plays (or, `edit`, builds) park `id`: open where it is here, or put down by hand.
fn open(world: &mut World, id: &str, edit: bool) -> Outcome {
    let (here, current) = {
        let parks = world.resource::<Parks>();
        (parks.here(id, world.resource::<Anchors>()), parks.is_current(id))
    };
    if current {
        world.resource_mut::<BuildMode>().active = edit;
        return Outcome::Close;
    }
    match here {
        Here::Looking => Outcome::Say("Still looking for it in this room: try again in a moment".into()),
        Here::Found if park::open_here(world, id) => {
            world.resource_mut::<BuildMode>().active = edit;
            Outcome::Close
        }
        _ => {
            if park::start_placing(world, id, edit) {
                Outcome::Close
            } else {
                Outcome::Say("That park couldn't be opened".into())
            }
        }
    }
}

/// The park cards' pictures, drawn again when a park is saved.
fn refresh_thumbs(world: &mut World) {
    let stale: Vec<(String, u64, ParkFile)> = {
        let parks = world.resource::<Parks>();
        let screen = world.resource::<ParksScreen>();
        parks.list.iter().filter(|p| screen.thumbs.get(&p.id).is_none_or(|(e, _)| *e != p.edits)).map(|p| (p.id.clone(), p.edits, p.file.clone())).collect()
    };
    for (id, edits, file) in stale {
        let image = image_of(thumbnail(&file, THUMB.0, THUMB.1), THUMB.0, THUMB.1);
        let handle = world.resource_mut::<Assets<Image>>().add(image);
        world.resource_mut::<ParksScreen>().thumbs.insert(id, (edits, handle));
    }
}

/// One frame of the parks screen: what the stick, the laser and the buttons did.
pub(super) fn frame(world: &mut World, nav: Nav, point: &UiPointer) -> Outcome {
    refresh_thumbs(world);
    let now = park::now();
    let tab = world.resource::<ParksScreen>().tab.min(TABS.len() - 1);
    let mode = TABS[tab];
    let items: Vec<Item> = {
        let parks = world.resource::<Parks>();
        let anchors = world.resource::<Anchors>();
        let screen = world.resource::<ParksScreen>();
        std::iter::once(Item::New(mode))
            .chain(parks.sorted(anchors).into_iter().filter(|&(i, _)| parks.list[i].file.mode() == Some(mode)).map(|(i, here)| {
                let p = &parks.list[i];
                let current = parks.is_current(&p.id);
                Item::Park(CardView {
                    id: p.id.clone(),
                    name: p.file.name.clone(),
                    sub: subtitle(&p.file, current, now),
                    here,
                    current,
                    thumb: screen.thumbs.get(&p.id).map(|(_, h)| h.clone()),
                    empty: p.file.pieces.is_empty(),
                    engine: p.file.engine().map(Engine::name),
                })
            }))
            .collect()
    };
    let lines = lines(&items);
    let park_id = |i: usize| match items.get(i) {
        Some(Item::Park(c)) => Some(c.id.clone()),
        _ => None,
    };
    // Under the laser: the smallest thing it is on (a button over its card).
    let hovered = point.px().and_then(|px| {
        let mut hits = world.query::<(&Hit, &ComputedNode, &UiGlobalTransform)>();
        hits.iter(world)
            .filter(|(_, node, tf)| node.size().x > 0.0 && node.contains_point(**tf, px))
            .min_by(|a, b| (a.1.size().x * a.1.size().y).total_cmp(&(b.1.size().x * b.1.size().y)))
            .map(|(hit, ..)| *hit)
    });
    let pressed = |hit: Option<Hit>| if point.click { hit } else { None };
    let mut outcome = Outcome::Stay;

    let (menu, confirm, warning, dropdown, settings) = {
        let screen = world.resource::<ParksScreen>();
        (screen.menu.clone(), screen.confirm.clone(), screen.warning.clone(), screen.engines, screen.settings.clone())
    };
    let rows = engine_rows(world, mode);
    if let Some((id, then, play)) = warning {
        // Its engine isn't installed: Cancel / Play with current engine.
        let mut play = play;
        if nav.left || nav.right || nav.up || nav.down {
            play = !play;
        }
        if let Some(Hit::Warn(p)) = hovered {
            play = p;
        }
        let chosen = match pressed(hovered) {
            Some(Hit::Warn(p)) => Some(p),
            Some(_) => Some(false),
            None if nav.select => Some(play),
            None if nav.back => Some(false),
            None => None,
        };
        match chosen {
            Some(true) => {
                world.resource_mut::<ParksScreen>().warning = None;
                outcome = carry_out(world, &id, then);
            }
            Some(false) => world.resource_mut::<ParksScreen>().warning = None,
            None => world.resource_mut::<ParksScreen>().warning = Some((id, then, play)),
        }
    } else if let (Some(row), Some(rows)) = (dropdown, rows.as_ref()) {
        // The engine dropdown: a row switches to its engine (a greyed one says why it can't).
        let mut row = row.min(rows.len() - 1);
        if nav.up {
            row = (row + rows.len() - 1) % rows.len();
        }
        if nav.down {
            row = (row + 1) % rows.len();
        }
        if let Some(Hit::EngineRow(r)) = hovered {
            row = r.min(rows.len() - 1);
        }
        let picked = match pressed(hovered) {
            Some(Hit::EngineRow(r)) => Some(Some(r)),
            Some(_) => Some(None),
            None if nav.select => Some(Some(row)),
            None if nav.back || nav.more => Some(None),
            None => None,
        };
        world.resource_mut::<ParksScreen>().engines = match picked {
            Some(Some(r)) if !rows[r].installed => {
                outcome = Outcome::Say(format!("{}: {}", rows[r].name, rows[r].why));
                Some(r)
            }
            Some(Some(r)) => {
                let engine = rows[r].engine;
                if !rows[r].running {
                    world.resource_mut::<Engines>().request(engine);
                    outcome = Outcome::Say(format!("Switched to {}", engine.name()));
                }
                None
            }
            Some(None) => None,
            None => Some(row),
        };
    } else if let Some((engine, row)) = world.resource::<ParksScreen>().engine_settings {
        // An engine's settings: its toggles (A, or the laser, flips one), Done.
        let toggles = EngineToggles::of(world, engine);
        let rows = toggles.len() + 1;
        let mut row = row.min(rows - 1);
        if nav.up {
            row = (row + rows - 1) % rows;
        }
        if nav.down {
            row = (row + 1) % rows;
        }
        match hovered {
            Some(Hit::Toggle(i)) => row = i,
            Some(Hit::SettingsDone) => row = rows - 1,
            _ => {}
        }
        let flip = match pressed(hovered) {
            Some(Hit::Toggle(i)) => Some(i),
            None if nav.select && row < toggles.len() => Some(row),
            _ => None,
        };
        if let Some(i) = flip
            && let Some((toggle, on)) = toggles.get(i)
        {
            EngineToggles::set(world, engine, toggle.key, !on);
        }
        let close = match pressed(hovered) {
            Some(Hit::SettingsDone | Hit::Backdrop) => true,
            Some(_) => false,
            None => (nav.select && row == rows - 1) || nav.back || nav.more,
        };
        world.resource_mut::<ParksScreen>().engine_settings = if close { None } else { Some((engine, row)) };
    } else if let Some((id, row)) = settings {
        // A course's settings: its size and its engine's settings stepped (left/right on its row, or the laser's -/+,
        // held to repeat), Done.
        let steppers = park::course_steppers(world, &id);
        let mut row = row;
        if let SettingsRow::Engine(i) = row
            && i >= steppers.len()
        {
            row = SettingsRow::Size;
        }
        if nav.up || nav.down {
            row = row.moved(nav.up, steppers.len());
        }
        match hovered {
            Some(Hit::SizeStep(_)) => row = SettingsRow::Size,
            Some(Hit::EngineStep(i, _)) => row = SettingsRow::Engine(i),
            Some(Hit::SettingsDone) => row = SettingsRow::Done,
            _ => {}
        }
        let dt = world.resource::<Time<Real>>().delta_secs();
        let mut steps = 0;
        if row != SettingsRow::Done {
            steps += i32::from(nav.right) - i32::from(nav.left);
        }
        let held_on = match hovered {
            Some(Hit::SizeStep(up) | Hit::EngineStep(_, up)) if point.held || point.click => Some(up),
            _ => None,
        };
        {
            let mut screen = world.resource_mut::<ParksScreen>();
            if super::repeat(&mut screen.step_held, held_on.is_some(), dt) {
                steps += if held_on == Some(true) { 1 } else { -1 };
            }
        }
        if steps != 0 {
            match row {
                SettingsRow::Size => {
                    let t = size::stepped(size::course_times(world, &id), steps);
                    size::set_course_times(world, &id, t);
                }
                SettingsRow::Engine(i) => {
                    let stepper = steppers[i];
                    let value = stepper.stepped(park::stepper_value(world, &id, &stepper), steps);
                    park::set_stepper(world, &id, &stepper, value);
                }
                SettingsRow::Done => {}
            }
        }
        let close = match pressed(hovered) {
            Some(Hit::SettingsDone | Hit::Backdrop) => true,
            Some(_) => false,
            None => (nav.select && row == SettingsRow::Done) || nav.back || nav.more,
        };
        world.resource_mut::<ParksScreen>().settings = if close { None } else { Some((id, row)) };
    } else if let Some((id, delete)) = confirm {
        // Delete it? Delete / Keep it.
        let mut delete = delete;
        if nav.left || nav.right || nav.up || nav.down {
            delete = !delete;
        }
        if let Some(Hit::Confirm(d)) = hovered {
            delete = d;
        }
        let chosen = match pressed(hovered) {
            Some(Hit::Confirm(d)) => Some(d),
            Some(_) => Some(false),
            None if nav.select => Some(delete),
            None if nav.back => Some(false),
            None => None,
        };
        match chosen {
            Some(true) => {
                park::delete(world, &id);
                let mut screen = world.resource_mut::<ParksScreen>();
                screen.confirm = None;
                // (its details were open: to the parks)
                if screen.details.as_ref().is_some_and(|d| d.id == id) {
                    screen.details = None;
                }
                outcome = Outcome::Say("Park deleted".into());
            }
            Some(false) => world.resource_mut::<ParksScreen>().confirm = None,
            None => world.resource_mut::<ParksScreen>().confirm = Some((id, delete)),
        }
    } else if let Some((id, row)) = menu {
        let (here, current) = {
            let parks = world.resource::<Parks>();
            (parks.here(&id, world.resource::<Anchors>()), parks.is_current(&id))
        };
        let rows = menu_for(here, current);
        let mut row = row.min(rows.len() - 1);
        if nav.up {
            row = (row + rows.len() - 1) % rows.len();
        }
        if nav.down {
            row = (row + 1) % rows.len();
        }
        if let Some(Hit::MenuRow(r)) = hovered {
            row = r.min(rows.len() - 1);
        }
        let action = match pressed(hovered) {
            Some(Hit::MenuRow(r)) => rows.get(r).copied(),
            Some(_) => {
                world.resource_mut::<ParksScreen>().menu = None;
                None
            }
            None if nav.select => Some(rows[row]),
            None if nav.back || nav.more => {
                world.resource_mut::<ParksScreen>().menu = None;
                None
            }
            None => {
                world.resource_mut::<ParksScreen>().menu = Some((id.clone(), row));
                None
            }
        };
        if let Some(action) = action {
            world.resource_mut::<ParksScreen>().menu = None;
            outcome = match action {
                MenuAction::Settings => {
                    world.resource_mut::<ParksScreen>().settings = Some((id, SettingsRow::Size));
                    Outcome::Stay
                }
                MenuAction::Duplicate => match park::duplicate(world, &id) {
                    Some(_) => Outcome::Say("Copied".into()),
                    None => Outcome::Stay,
                },
                MenuAction::PlaceElsewhere => open_with_engine(world, &id, Then::Place),
                MenuAction::Forget => {
                    park::forget_here(world, &id);
                    Outcome::Say("Forgotten in this room: it opens in its other places".into())
                }
                MenuAction::Delete => {
                    world.resource_mut::<ParksScreen>().confirm = Some((id, false));
                    Outcome::Stay
                }
            };
        }
    } else if world.resource::<ParksScreen>().details.is_some() {
        outcome = course::frame(world, nav, point);
    } else {
        let mut focus = world.resource::<ParksScreen>().focus;
        if let Focus::Item(i, _) = focus {
            if i >= items.len() {
                focus = Focus::Item(items.len() - 1, Part::Main);
            }
        }
        focus = moved(focus, &nav, &items, &lines);
        // (no gear without engine settings: the stick passes it)
        let gear = rows.as_ref().and_then(|r| r.iter().find(|e| e.running)).is_some_and(|e| !EngineToggles::of(world, e.engine).is_empty());
        if focus == Focus::EngineGear && !gear {
            focus = if nav.down { Focus::Item(0, Part::Main) } else { Focus::Engine };
        }
        // The laser chooses what it is on.
        match hovered {
            Some(Hit::Tab(t)) => focus = Focus::Tab(t),
            Some(Hit::Settings) => focus = Focus::Settings,
            Some(Hit::Engine) => focus = Focus::Engine,
            Some(Hit::EngineGear) => focus = Focus::EngineGear,
            Some(Hit::Item(i) | Hit::Play(i) | Hit::More(i)) => focus = Focus::Item(i, Part::Main),
            Some(Hit::Edit(i)) => focus = Focus::Item(i, Part::Edit),
            _ => {}
        }
        world.resource_mut::<ParksScreen>().focus = focus;
        let press = match pressed(hovered) {
            Some(hit) => Some(hit),
            None if nav.select => Some(match focus {
                Focus::Tab(t) => Hit::Tab(t),
                Focus::Settings => Hit::Settings,
                Focus::Engine => Hit::Engine,
                Focus::EngineGear => Hit::EngineGear,
                Focus::Item(i, Part::Main) => Hit::Item(i),
                Focus::Item(i, Part::Edit) => Hit::Edit(i),
            }),
            None if nav.more => match focus {
                Focus::Item(i, _) => Some(Hit::More(i)),
                Focus::Tab(_) | Focus::Settings | Focus::Engine | Focus::EngineGear => None,
            },
            None => None,
        };
        outcome = match press {
            Some(Hit::Tab(t)) => {
                let mut screen = world.resource_mut::<ParksScreen>();
                if screen.tab != t {
                    screen.tab = t;
                    screen.first = 0;
                }
                Outcome::Stay
            }
            Some(Hit::Settings) => Outcome::Open(pages::START),
            Some(Hit::EngineGear) => {
                let engine = rows.as_ref().and_then(|r| r.iter().find(|e| e.running)).map(|e| e.engine);
                if let Some(engine) = engine.filter(|&e| !EngineToggles::of(world, e).is_empty()) {
                    world.resource_mut::<ParksScreen>().engine_settings = Some((engine, 0));
                }
                Outcome::Stay
            }
            Some(Hit::Engine) => {
                let running = rows.as_ref().and_then(|r| r.iter().position(|e| e.running)).unwrap_or(0);
                world.resource_mut::<ParksScreen>().engines = Some(running);
                Outcome::Stay
            }
            Some(Hit::Item(0)) if matches!(items.first(), Some(Item::New(_))) => {
                park::new_park(world, mode);
                world.resource_mut::<BuildMode>().active = true;
                Outcome::Close
            }
            Some(Hit::Item(i) | Hit::Play(i)) => park_id(i).map_or(Outcome::Stay, |id| open_with_engine(world, &id, Then::Open(false))),
            Some(Hit::Edit(i)) => park_id(i).map_or(Outcome::Stay, |id| open_with_engine(world, &id, Then::Open(true))),
            Some(Hit::More(i)) => {
                if let Some(id) = park_id(i) {
                    world.resource_mut::<ParksScreen>().menu = Some((id, 0));
                }
                Outcome::Stay
            }
            _ if nav.back => Outcome::Back,
            _ => Outcome::Stay,
        };
    }

    // What to draw.
    let (focus, first, menu, confirm, warning, dropdown) = {
        let screen = world.resource::<ParksScreen>();
        (screen.focus, screen.first, screen.menu.clone(), screen.confirm.clone(), screen.warning.clone(), screen.engines)
    };
    let focus_line = match focus {
        Focus::Item(i, _) => line_of(&lines, i).map_or(0, |(l, _)| l),
        Focus::Tab(_) | Focus::Settings | Focus::Engine | Focus::EngineGear => 0,
    };
    let rows = engine_rows(world, mode);
    let engine_gear = rows.as_ref().and_then(|r| r.iter().find(|e| e.running)).is_some_and(|e| !EngineToggles::of(world, e.engine).is_empty());
    let engine = rows.map(|rows| {
        let running = rows.iter().find(|r| r.running).map_or_else(|| "…".to_string(), |r| r.name.clone());
        (running, dropdown.map(|row| (rows, row)))
    });
    let warning = warning.and_then(|(id, _, play)| match park::engine_fit(world, &id) {
        EngineFit::Missing(e) => Some((e.name().to_string(), play)),
        _ => None,
    });
    let first = scrolled(&lines, first, focus_line);
    let name = |id: &str| world.resource::<Parks>().get(id).map_or(String::new(), |p| p.file.name.clone());
    let menu = menu.map(|(id, row)| {
        let parks = world.resource::<Parks>();
        let rows = menu_for(parks.here(&id, world.resource::<Anchors>()), parks.is_current(&id));
        (name(&id), rows, row)
    });
    let confirm = confirm.map(|(id, delete)| (name(&id), delete));
    let settings = world.resource::<ParksScreen>().settings.clone().map(|(id, row)| {
        let t = size::course_times(world, &id);
        let steppers = park::course_steppers(world, &id)
            .iter()
            .map(|s| {
                let v = park::stepper_value(world, &id, s);
                StepperView { label: s.label.into(), help: s.help.into(), value: s.label_of(v), down: s.stepped(v, -1) < v, up: s.stepped(v, 1) > v }
            })
            .collect();
        SettingsView {
            title: "Course settings".into(),
            name: name(&id),
            size: size::label(t),
            smaller: size::stepped(t, -1) < t,
            bigger: size::stepped(t, 1) > t,
            steppers,
            row,
        }
    });
    let status = {
        let ui = world.resource::<XrUi>();
        if ui.status.1 > 0.0 { ui.status.0.clone() } else { String::new() }
    };
    let engine_settings = world.resource::<ParksScreen>().engine_settings.map(|(engine, row)| EngineSettingsView {
        name: engine.name().to_string(),
        toggles: EngineToggles::of(world, engine).into_iter().map(|(t, on)| (t.label.to_string(), t.help.to_string(), on)).collect(),
        row,
    });
    let details = course::view(world, settings.is_some() || confirm.is_some() || engine_settings.is_some());
    let model = Model { tab, items, lines, first, focus: Some(focus), menu, confirm, settings, engine_settings, engine_gear, details, engine, warning, status };
    let mut screen = world.resource_mut::<ParksScreen>();
    screen.first = first;
    screen.model = model;
    outcome
}

// ---------------------------------------------------------------- pictures

fn image_of(pixels: Vec<u8>, w: u32, h: u32) -> Image {
    Image::new(Extent3d { width: w, height: h, depth_or_array_layers: 1 }, TextureDimension::D2, pixels, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD)
}

fn rgb(c: Color) -> [f32; 3] {
    let c = c.to_srgba();
    [c.red, c.green, c.blue]
}

/// Fills the triangle `t` (pixel coordinates) in `pixels` (w x h, RGBA).
fn fill(pixels: &mut [u8], w: u32, h: u32, t: [Vec2; 3], color: [f32; 3]) {
    let lo = t[0].min(t[1]).min(t[2]).max(Vec2::ZERO);
    let hi = t[0].max(t[1]).max(t[2]).min(Vec2::new(w as f32 - 1.0, h as f32 - 1.0));
    let area = (t[1] - t[0]).perp_dot(t[2] - t[0]);
    if area.abs() < 1e-6 || hi.x < lo.x || hi.y < lo.y {
        return;
    }
    for y in lo.y.floor() as u32..=hi.y.ceil() as u32 {
        for x in lo.x.floor() as u32..=hi.x.ceil() as u32 {
            if x >= w || y >= h {
                continue;
            }
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let a = (t[1] - p).perp_dot(t[2] - p) / area;
            let b = (t[2] - p).perp_dot(t[0] - p) / area;
            if a >= 0.0 && b >= 0.0 && a + b <= 1.0 {
                let i = ((y * w + x) * 4) as usize;
                for (k, c) in color.iter().enumerate() {
                    pixels[i + k] = (c * 255.0).round().clamp(0.0, 255.0) as u8;
                }
                pixels[i + 3] = 255;
            }
        }
    }
}

/// A park from above: its pieces' upward faces in their kinds' colours,
/// toned down to the panel's slate and lighter the higher they are, on a
/// faint grid of real metres (at the scale it was last open at).
fn thumbnail(file: &ParkFile, w: u32, h: u32) -> Vec<u8> {
    let background = rgb(THUMB_BG);
    let mut pixels: Vec<u8> = (0..w * h).flat_map(|_| background.map(|c| (c * 255.0).round() as u8).into_iter().chain([255])).collect();
    let slate = [0.2, 0.27, 0.29];
    let mut faces: Vec<([Vec3; 3], [f32; 3])> = Vec::new();
    for piece in &file.pieces {
        let k = file.park_scale * piece.scale;
        let (at, turn) = (Vec3::from_array(piece.position), Quat::from_rotation_y(piece.yaw));
        let own = rgb(piece.piece.kind.color());
        let color = [0, 1, 2].map(|i| slate[i] * 0.45 + own[i] * 0.55);
        for q in piece.geometry().all_quads() {
            let q = q.map(|p| at + turn * (p * k));
            for t in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                if (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero().y.abs() > 0.3 {
                    faces.push((t, color));
                }
            }
        }
    }
    if faces.is_empty() {
        return pixels;
    }
    let (lo, hi) = faces.iter().flat_map(|(t, _)| t.iter()).fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let span = (hi - lo).max(Vec3::splat(1e-3));
    let margin = 0.12;
    let fit = ((w as f32 * (1.0 - 2.0 * margin)) / span.x).min((h as f32 * (1.0 - 2.0 * margin)) / span.z);
    let middle = (lo + hi) * 0.5;
    let to_px = |p: Vec3| Vec2::new(w as f32 * 0.5 + (p.x - middle.x) * fit, h as f32 * 0.5 + (p.z - middle.z) * fit);
    // A metre of the room, as faint lines.
    let metre = file.scale.max(1e-3) * fit;
    if metre > 8.0 {
        let grid = [0.14, 0.2, 0.225];
        let origin = to_px(Vec3::ZERO);
        let mut x = origin.x.rem_euclid(metre);
        while x < w as f32 {
            fill(&mut pixels, w, h, [Vec2::new(x, 0.0), Vec2::new(x + 1.0, 0.0), Vec2::new(x, h as f32)], grid);
            fill(&mut pixels, w, h, [Vec2::new(x + 1.0, 0.0), Vec2::new(x + 1.0, h as f32), Vec2::new(x, h as f32)], grid);
            x += metre;
        }
        let mut y = origin.y.rem_euclid(metre);
        while y < h as f32 {
            fill(&mut pixels, w, h, [Vec2::new(0.0, y), Vec2::new(w as f32, y), Vec2::new(0.0, y + 1.0)], grid);
            fill(&mut pixels, w, h, [Vec2::new(w as f32, y), Vec2::new(w as f32, y + 1.0), Vec2::new(0.0, y + 1.0)], grid);
            y += metre;
        }
    }
    // Lowest first, so what is higher is drawn over it.
    let mean = |t: &[Vec3; 3]| t.iter().map(|p| p.y).sum::<f32>() / 3.0;
    faces.sort_by(|a, b| mean(&a.0).total_cmp(&mean(&b.0)));
    for (t, color) in faces {
        let rise = ((mean(&t) - lo.y) / span.y.max(1e-3)).clamp(0.0, 1.0);
        let shade = 0.62 + 0.38 * rise;
        fill(&mut pixels, w, h, t.map(to_px), color.map(|c| c * shade));
    }
    pixels
}

/// The icons, white on clear (tinted where they are drawn): play, a pencil,
/// three dots, a gear.
fn icons() -> [Vec<u8>; 9] {
    let n = ICON as f32;
    let shape = |inside: &dyn Fn(Vec2) -> bool| -> Vec<u8> {
        let mut out = vec![0u8; (ICON * ICON * 4) as usize];
        for y in 0..ICON {
            for x in 0..ICON {
                // (4 x 4 samples a pixel: smooth edges)
                let covered = (0..16).filter(|s| inside(Vec2::new(x as f32 + (s % 4) as f32 * 0.25 + 0.125, y as f32 + (s / 4) as f32 * 0.25 + 0.125) / n)).count();
                let i = ((y * ICON + x) * 4) as usize;
                out[i..i + 3].copy_from_slice(&[255, 255, 255]);
                out[i + 3] = (covered * 255 / 16) as u8;
            }
        }
        out
    };
    let triangle = |p: Vec2, a: Vec2, b: Vec2, c: Vec2| {
        let s = |p0: Vec2, p1: Vec2| (p1 - p0).perp_dot(p - p0);
        let (d0, d1, d2) = (s(a, b), s(b, c), s(c, a));
        (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0) || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0)
    };
    let play = shape(&|p| triangle(p, Vec2::new(0.3, 0.18), Vec2::new(0.3, 0.82), Vec2::new(0.84, 0.5)));
    let pencil = shape(&|p| {
        // A bar from the top right down to a point at the bottom left.
        let (from, to) = (Vec2::new(0.8, 0.2), Vec2::new(0.33, 0.67));
        let along = (to - from).normalize();
        let (t, across, length) = ((p - from).dot(along), (p - from).perp_dot(along).abs(), from.distance(to));
        (t >= 0.0 && t <= length && across <= 0.1) || (t > length && t <= length + 0.16 && across <= 0.1 * (1.0 - (t - length) / 0.16))
    });
    let more = shape(&|p| [0.25, 0.5, 0.75].iter().any(|&x| p.distance(Vec2::new(x, 0.5)) <= 0.075));
    let gear = shape(&|p| {
        let d = p - Vec2::splat(0.5);
        let (r, a) = (d.length(), d.y.atan2(d.x));
        // Eight teeth round a wheel, and its hole.
        let tooth = ((a * 8.0 / std::f32::consts::TAU).rem_euclid(1.0) - 0.5).abs() < 0.22;
        let outer = if tooth { 0.44 } else { 0.34 };
        r <= outer && r >= 0.13
    });
    // (a stroke from `a` to `b`, `w` wide each side)
    let stroke = |p: Vec2, a: Vec2, b: Vec2, w: f32| {
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
        p.distance(a + ab * t) <= w
    };
    let back = shape(&|p| {
        let (l, r) = (Vec2::new(0.2, 0.5), Vec2::new(0.8, 0.5));
        stroke(p, l, r, 0.06) || stroke(p, l, Vec2::new(0.45, 0.25), 0.06) || stroke(p, l, Vec2::new(0.45, 0.75), 0.06)
    });
    let trash = shape(&|p| {
        let lid = stroke(p, Vec2::new(0.2, 0.27), Vec2::new(0.8, 0.27), 0.05) || stroke(p, Vec2::new(0.4, 0.17), Vec2::new(0.6, 0.17), 0.05);
        let sides = stroke(p, Vec2::new(0.28, 0.35), Vec2::new(0.33, 0.83), 0.05) || stroke(p, Vec2::new(0.72, 0.35), Vec2::new(0.67, 0.83), 0.05);
        let base = stroke(p, Vec2::new(0.33, 0.83), Vec2::new(0.67, 0.83), 0.05);
        let lines = stroke(p, Vec2::new(0.43, 0.42), Vec2::new(0.43, 0.72), 0.035) || stroke(p, Vec2::new(0.57, 0.42), Vec2::new(0.57, 0.72), 0.035);
        lid || sides || base || lines
    });
    let chevron = shape(&|p| stroke(p, Vec2::new(0.22, 0.38), Vec2::new(0.5, 0.66), 0.08) || stroke(p, Vec2::new(0.5, 0.66), Vec2::new(0.78, 0.38), 0.08));
    let sliders = shape(&|p| {
        let bars = stroke(p, Vec2::new(0.15, 0.32), Vec2::new(0.85, 0.32), 0.04) || stroke(p, Vec2::new(0.15, 0.68), Vec2::new(0.85, 0.68), 0.04);
        bars || p.distance(Vec2::new(0.64, 0.32)) <= 0.11 || p.distance(Vec2::new(0.38, 0.68)) <= 0.11
    });
    let check = shape(&|p| stroke(p, Vec2::new(0.2, 0.52), Vec2::new(0.42, 0.74), 0.08) || stroke(p, Vec2::new(0.42, 0.74), Vec2::new(0.82, 0.3), 0.08));
    [play, pencil, more, gear, back, trash, chevron, sliders, check]
}

// ---------------------------------------------------------------- view

pub(super) fn spawn(mut commands: Commands, panel: Res<XrPanel>, assets: Res<AssetServer>, mut images: ResMut<Assets<Image>>, mut screen: ResMut<ParksScreen>) {
    screen.fonts = Some(Fonts { display: assets.load(UI_FONT), body: assets.load(UI_BODY_FONT) });
    let [play, pencil, more, gear, back, trash, chevron, sliders, check] = icons().map(|pixels| images.add(image_of(pixels, ICON, ICON)));
    screen.icons = Some(Icons { play, pencil, more, gear, back, trash, chevron, sliders, check });
    let root = commands
        .spawn((
            ParksRoot,
            Node { width: percent(100), height: percent(100), position_type: PositionType::Absolute, display: Display::None, ..default() },
            GlobalZIndex(20),
            UiTargetCamera(panel.camera),
        ))
        .id();
    screen.root = Some(root);
}

type Spawner<'a> = bevy::ecs::relationship::RelatedSpawner<'a, ChildOf>;

/// A focus ring, `gap` outside the edge.
fn ring(on: bool, width: f32, gap: f32) -> Outline {
    if on { Outline { width: px(width), offset: px(gap), color: FOCUS } } else { Outline { width: px(0), offset: px(0), color: Color::NONE } }
}

fn icon(parent: &mut Spawner, image: &Handle<Image>, size: f32, color: Color) {
    parent.spawn((ImageNode { color, ..ImageNode::new(image.clone()) }, Node { width: px(size), height: px(size), flex_shrink: 0.0, ..default() }));
}

/// A name that fits its card: a long one ends in "…".
fn fitted(name: &str) -> String {
    if name.chars().count() <= NAME_LETTERS {
        name.to_string()
    } else {
        format!("{}…", name.chars().take(NAME_LETTERS - 1).collect::<String>().trim_end())
    }
}

/// Small capitals, spaced a little apart.
fn spaced_caps(text: &str) -> String {
    text.to_uppercase().chars().map(|c| c.to_string()).collect::<Vec<_>>().join("\u{200A}")
}

/// The screen drawn again when it changed.
pub(super) fn draw(world: &mut World) {
    let show = world.resource::<XrUi>().on_parks();
    let Some(root) = world.resource::<ParksScreen>().root else { return };
    {
        let Some(mut node) = world.get_mut::<Node>(root) else { return };
        let display = if show { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
    }
    if !show {
        world.resource_mut::<ParksScreen>().drawn = None;
        return;
    }
    let (model, fonts, icons) = {
        let screen = world.resource::<ParksScreen>();
        if screen.drawn.as_ref() == Some(&screen.model) {
            return;
        }
        (screen.model.clone(), screen.fonts.clone(), screen.icons.clone())
    };
    let (Some(fonts), Some(icons)) = (fonts, icons) else { return };
    world.resource_mut::<ParksScreen>().drawn = Some(model.clone());
    let dropdown_open = model.engine.as_ref().is_some_and(|(_, d)| d.is_some());
    let modal = model.menu.is_some() || model.confirm.is_some() || model.warning.is_some() || model.engine_settings.is_some() || dropdown_open;
    let focus = model.focus.filter(|_| !modal);
    world.entity_mut(root).despawn_children();
    world.entity_mut(root).with_children(|root| {
        if let Some(details) = &model.details {
            course::panel(root, details, &fonts, &icons);
        } else {
        root.spawn((
            Node {
                width: percent(100),
                height: percent(100),
                padding: UiRect::axes(px(40), px(36)),
                flex_direction: FlexDirection::Column,
                row_gap: px(26),
                border_radius: BorderRadius::all(px(24)),
                ..default()
            },
            BackgroundColor(PANEL),
        ))
        .with_children(|panel| {
            // The title, and the settings gear in the corner.
            panel.spawn(Node { width: percent(100), height: px(64), flex_shrink: 0.0, align_items: AlignItems::Center, column_gap: px(14), ..default() }).with_children(|bar| {
                // The tabs: each mode's courses.
                for (t, &mode) in TABS.iter().enumerate() {
                    let on = t == model.tab;
                    bar.spawn((
                        Hit::Tab(t),
                        Node { padding: UiRect::new(px(6), px(6), px(4), px(6)), border: UiRect::bottom(px(4)), ..default() },
                        BorderColor::all(if on { TEAL } else { Color::NONE }),
                        ring(focus == Some(Focus::Tab(t)), 3.5, 4.0),
                    ))
                    .with_children(|tab| {
                        tab.spawn((Text::new(park::courses_name(mode)), fonts.display(40.0, 800), TextColor(if on { Color::WHITE } else { OFF })));
                    });
                }
                bar.spawn(Node { flex_grow: 1.0, ..default() });
                // The engine: a dropdown that switches it (none for a mode with no engine in this app).
                if let Some((running, dropdown)) = &model.engine {
                    engine_pill(bar, running, dropdown.is_some(), focus == Some(Focus::Engine), &fonts, &icons);
                    if model.engine_gear {
                        engine_gear(bar, Hit::EngineGear, focus == Some(Focus::EngineGear), &icons);
                    }
                }
            });
            // The cards, and how far down them it is.
            panel.spawn(Node { width: percent(100), flex_grow: 1.0, flex_direction: FlexDirection::Column, row_gap: px(GAP_PX), ..default() }).with_children(|cards| {
                let count = shown(&model.lines, model.first);
                for line in model.lines.iter().skip(model.first).take(count) {
                    match line {
                        Line::Divider(label) => {
                            cards.spawn(Node { width: percent(100), height: px(DIVIDER_PX), flex_shrink: 0.0, align_items: AlignItems::Center, column_gap: px(14), ..default() }).with_children(|d| {
                                d.spawn((Text::new(spaced_caps(label)), fonts.body(16.0, 500), TextColor(OFF)));
                                d.spawn((Node { flex_grow: 1.0, height: px(1), ..default() }, BackgroundColor(LINE)));
                            });
                        }
                        Line::Cards(items) => {
                            cards.spawn(Node { width: percent(100), flex_shrink: 0.0, column_gap: px(GAP_PX), ..default() }).with_children(|row| {
                                for &i in items {
                                    card(row, i, &model.items[i], focus, &icons, &fonts);
                                }
                            });
                        }
                    }
                }
                if model.first > 0 || model.first + count < model.lines.len() {
                    let total: f32 = model.lines.iter().map(height).sum();
                    let above: f32 = model.lines[..model.first].iter().map(height).sum();
                    let seen: f32 = model.lines[model.first..model.first + count].iter().map(height).sum();
                    cards
                        .spawn((
                            Node { position_type: PositionType::Absolute, right: px(-24), top: px(0), bottom: px(GAP_PX), width: px(4), border_radius: BorderRadius::all(px(2)), ..default() },
                            BackgroundColor(LINE),
                        ))
                        .with_children(|track| {
                            track.spawn((
                                Node { position_type: PositionType::Absolute, left: px(0), right: px(0), top: percent(above / total * 100.0), height: percent(seen / total * 100.0), border_radius: BorderRadius::all(px(2)), ..default() },
                                BackgroundColor(TEAL.with_alpha(0.7)),
                            ));
                        });
                }
            });
            // What the buttons do.
            panel
                .spawn((
                    Node { width: percent(100), flex_shrink: 0.0, border: UiRect::top(px(1)), padding: UiRect::top(px(14)), column_gap: px(24), align_items: AlignItems::Center, ..default() },
                    BorderColor::all(LINE),
                ))
                .with_children(|foot| {
                    foot.spawn(Node { flex_grow: 1.0, flex_shrink: 1.0, column_gap: px(30), row_gap: px(12), flex_wrap: FlexWrap::Wrap, align_items: AlignItems::Center, ..default() }).with_children(|legend| {
                        for (control, what) in [("Trigger", "play · pencil builds"), ("Grip", "more"), ("B / Menu", "back to playing"), ("Stick", "move")] {
                            legend.spawn(Node { column_gap: px(7), ..default() }).with_children(|l| {
                                l.spawn((Text::new(control), fonts.body(17.5, 600), TextColor(DIM)));
                                l.spawn((Text::new(what), fonts.body(17.5, 400), TextColor(OFF)));
                            });
                        }
                    });
                    // The app's own menu, bottom right.
                    foot.spawn((
                        Hit::Settings,
                        Node {
                            height: px(56),
                            flex_shrink: 0.0,
                            padding: UiRect::axes(px(20), px(0)),
                            border: UiRect::all(px(1)),
                            border_radius: BorderRadius::all(px(16)),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BorderColor::all(LINE),
                        ring(focus == Some(Focus::Settings), 3.5, 4.0),
                    ))
                    .with_children(|b| {
                        b.spawn((Text::new("buildAR Menu"), fonts.body(19.0, 600), TextColor(DIM), TextLayout::new_with_no_wrap()));
                    });
                });
        });
        // The engine dropdown's list, over the cards under its pill.
        if let Some((_, Some((rows, chosen)))) = &model.engine {
            engine_list(root, rows, *chosen, &fonts);
        }
        }
        // What was just done, over the bottom of the cards.
        if !model.status.is_empty() {
            root.spawn(Node { position_type: PositionType::Absolute, left: px(0), right: px(0), bottom: px(104), justify_content: JustifyContent::Center, ..default() }).with_children(|toast| {
                toast
                    .spawn((Node { padding: UiRect::axes(px(24), px(11)), border_radius: BorderRadius::all(px(99)), ..default() }, BackgroundColor(TEAL)))
                    .with_children(|t| {
                        t.spawn((Text::new(model.status.clone()), fonts.body(19.0, 600), TextColor(INK)));
                    });
            });
        }
        if let Some((name, rows, chosen)) = &model.menu {
            modal_box(root, 420.0, |b| {
                b.spawn((Text::new(fitted(name)), fonts.display(22.0, 700), TextColor(Color::WHITE), Node { padding: UiRect::new(px(18), px(18), px(10), px(8)), ..default() }));
                for (k, action) in rows.iter().enumerate() {
                    let danger = *action == MenuAction::Delete;
                    if danger {
                        b.spawn((Node { height: px(1), margin: UiRect::axes(px(6), px(6)), ..default() }, BackgroundColor(LINE)));
                    }
                    let on = k == *chosen;
                    b.spawn((
                        Hit::MenuRow(k),
                        Node { width: percent(100), padding: UiRect::axes(px(18), px(12)), border_radius: BorderRadius::all(px(10)), ..default() },
                        BackgroundColor(if on { SELECTED } else { Color::NONE }),
                    ))
                    .with_children(|r| {
                        let color = if danger { DANGER } else if on { Color::WHITE } else { DIM };
                        r.spawn((Text::new(action.label()), fonts.body(21.0, 500), TextColor(color)));
                    });
                }
            });
        }
        if let Some(view) = &model.engine_settings {
            engine_settings_box(root, view, &fonts);
        }
        if let Some(view) = &model.settings {
            settings_box(root, view, &fonts);
        }
        if let Some((engine, play)) = &model.warning {
            modal_box(root, 640.0, |b| {
                b.spawn(Node { flex_direction: FlexDirection::Column, row_gap: px(12), padding: UiRect::new(px(18), px(18), px(14), px(6)), ..default() }).with_children(|words| {
                    words.spawn((Text::new("WARNING"), fonts.display(28.0, 800), TextColor(DANGER)));
                    words.spawn((
                        Text::new(format!("This park was designed for the {engine} engine, which is not installed.")),
                        fonts.body(21.0, 400),
                        TextColor(DIM),
                    ));
                });
                b.spawn(Node { column_gap: px(14), padding: UiRect::all(px(18)), justify_content: JustifyContent::FlexEnd, ..default() }).with_children(|buttons| {
                    for (yes, label) in [(false, "Cancel"), (true, "Play with current engine")] {
                        let on = *play == yes;
                        buttons
                            .spawn((
                                Hit::Warn(yes),
                                Node { padding: UiRect::axes(px(26), px(12)), border_radius: BorderRadius::all(px(99)), border: UiRect::all(px(1)), ..default() },
                                BackgroundColor(if on { TEAL } else { Color::NONE }),
                                BorderColor::all(if on { TEAL } else { LINE }),
                            ))
                            .with_children(|b| {
                                b.spawn((Text::new(label), fonts.display(22.0, 700), TextColor(if on { INK } else { DIM })));
                            });
                    }
                });
            });
        }
        if let Some((name, delete)) = &model.confirm {
            modal_box(root, 560.0, |b| {
                b.spawn(Node { flex_direction: FlexDirection::Column, row_gap: px(10), padding: UiRect::new(px(18), px(18), px(14), px(6)), ..default() }).with_children(|words| {
                    words.spawn((Text::new(format!("Delete \u{201c}{}\u{201d}?", fitted(name))), fonts.display(28.0, 800), TextColor(Color::WHITE)));
                    words.spawn((Text::new("This can't be undone."), fonts.body(19.0, 400), TextColor(DIM)));
                });
                b.spawn(Node { column_gap: px(14), padding: UiRect::all(px(18)), justify_content: JustifyContent::FlexEnd, ..default() }).with_children(|buttons| {
                    for (yes, label) in [(false, "Keep it"), (true, "Delete")] {
                        let on = *delete == yes;
                        let tone = if yes { DANGER } else { TEAL };
                        buttons
                            .spawn((
                                Hit::Confirm(yes),
                                Node { padding: UiRect::axes(px(26), px(12)), border_radius: BorderRadius::all(px(99)), border: UiRect::all(px(1)), ..default() },
                                BackgroundColor(if on { tone } else { Color::NONE }),
                                BorderColor::all(if on { tone } else { LINE }),
                            ))
                            .with_children(|b| {
                                b.spawn((Text::new(label), fonts.display(22.0, 700), TextColor(if on { INK } else { DIM })));
                            });
                    }
                });
            });
        }
    });
}

/// Where the engine dropdown's list is on the panel: under its pill, beside the gear (px from the panel's right and
/// top: its padding, the gear and the gap between them).
const ENGINE_LIST_AT: (f32, f32) = (40.0 + 64.0 + 20.0, 100.0);

/// The engine's pill beside the title: the running engine's name and an arrow; pressed, it opens the list.
fn engine_pill(bar: &mut Spawner, running: &str, open: bool, focused: bool, fonts: &Fonts, icons: &Icons) {
    bar.spawn((
        Hit::Engine,
        Node {
            height: px(48),
            flex_shrink: 0.0,
            padding: UiRect::axes(px(18), px(0)),
            column_gap: px(10),
            align_items: AlignItems::Center,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(99)),
            ..default()
        },
        BackgroundColor(if open { SELECTED } else { Color::NONE }),
        BorderColor::all(LINE),
        ring(focused, 3.5, 4.0),
    ))
    .with_children(|pill| {
        pill.spawn((Text::new("Engine"), fonts.body(18.0, 500), TextColor(OFF)));
        pill.spawn((Text::new(running), fonts.display(22.0, 700), TextColor(Color::WHITE), TextLayout::new_with_no_wrap()));
        // (the play triangle turned to point down)
        pill.spawn((
            ImageNode { color: DIM, ..ImageNode::new(icons.play.clone()) },
            Node { width: px(14), height: px(14), flex_shrink: 0.0, ..default() },
            UiTransform::from_rotation(Rot2::degrees(90.0)),
        ));
    });
}

/// The engines under the pill: the running one ticked, the ones not installed greyed out with why.
fn engine_list(root: &mut Spawner, rows: &[EngineRow], chosen: usize, fonts: &Fonts) {
    root.spawn((
        Node {
            position_type: PositionType::Absolute,
            right: px(ENGINE_LIST_AT.0),
            top: px(ENGINE_LIST_AT.1),
            width: px(440),
            padding: UiRect::all(px(8)),
            flex_direction: FlexDirection::Column,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(14)),
            ..default()
        },
        BackgroundColor(MENU),
        BorderColor::all(LINE),
    ))
    .with_children(|list| {
        for (k, row) in rows.iter().enumerate() {
            let on = k == chosen;
            list.spawn((
                Hit::EngineRow(k),
                Node { width: percent(100), padding: UiRect::axes(px(16), px(11)), flex_direction: FlexDirection::Column, row_gap: px(3), border_radius: BorderRadius::all(px(10)), ..default() },
                BackgroundColor(if on { SELECTED } else { Color::NONE }),
            ))
            .with_children(|r| {
                let grey = OFF.with_alpha(0.55);
                let color = if !row.installed { grey } else if on || row.running { Color::WHITE } else { DIM };
                let label = if row.running { format!("{}  (running)", row.name) } else { row.name.clone() };
                r.spawn((Text::new(label), fonts.body(21.0, if row.running { 700 } else { 500 }), TextColor(color)));
                if !row.installed {
                    r.spawn((Text::new(format!("Not installed: {}", row.why)), fonts.body(15.0, 400), TextColor(grey)));
                }
            });
        }
    });
}

/// A box over the dimmed screen (pressing beside it closes it).
fn modal_box(root: &mut Spawner, width: f32, body: impl FnOnce(&mut Spawner)) {
    root.spawn((
        Hit::Backdrop,
        Node { width: percent(100), height: percent(100), position_type: PositionType::Absolute, align_items: AlignItems::Center, justify_content: JustifyContent::Center, border_radius: BorderRadius::all(px(24)), ..default() },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
    ))
    .with_children(|backdrop| {
        backdrop
            .spawn((
                Node { width: px(width), padding: UiRect::all(px(10)), flex_direction: FlexDirection::Column, border_radius: BorderRadius::all(px(16)), border: UiRect::all(px(1)), ..default() },
                BackgroundColor(MENU),
                BorderColor::all(LINE),
            ))
            .with_children(body);
    });
}

/// A course's settings, over the cards: its size, a stepper (-, the size, +), and Done.
fn settings_box(root: &mut Spawner, view: &SettingsView, fonts: &Fonts) {
    modal_box(root, 680.0, |b| {
        b.spawn(Node { flex_direction: FlexDirection::Column, row_gap: px(6), padding: UiRect::new(px(18), px(18), px(14), px(14)), ..default() }).with_children(|words| {
            words.spawn((Text::new(view.title.clone()), fonts.display(28.0, 800), TextColor(Color::WHITE)));
            words.spawn((Text::new(fitted(&view.name)), fonts.body(19.0, 500), TextColor(DIM)));
        });
        stepper_row(
            b,
            view.row == SettingsRow::Size,
            ("Player scale", "x1.0 is the default (toy-sized)", &view.size),
            (view.smaller, view.bigger),
            Hit::SizeStep,
            fonts,
        );
        for (i, s) in view.steppers.iter().enumerate() {
            stepper_row(b, view.row == SettingsRow::Engine(i), (&s.label, &s.help, &s.value), (s.down, s.up), move |up| Hit::EngineStep(i, up), fonts);
        }
        b.spawn(Node { padding: UiRect::all(px(18)), justify_content: JustifyContent::FlexEnd, ..default() }).with_children(|buttons| {
            let on = view.row == SettingsRow::Done;
            buttons
                .spawn((
                    Hit::SettingsDone,
                    Node { padding: UiRect::axes(px(30), px(12)), border_radius: BorderRadius::all(px(99)), border: UiRect::all(px(1)), ..default() },
                    BackgroundColor(if on { TEAL } else { Color::NONE }),
                    BorderColor::all(if on { TEAL } else { LINE }),
                ))
                .with_children(|b| {
                    b.spawn((Text::new("Done"), fonts.display(22.0, 700), TextColor(if on { INK } else { DIM })));
                });
        });
    });
}

/// The small gear by an engine's dropdown: its engine settings.
fn engine_gear<H: Component>(bar: &mut Spawner, hit: H, focused: bool, icons: &Icons) {
    bar.spawn((
        hit,
        Node {
            width: px(48),
            height: px(48),
            flex_shrink: 0.0,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(24)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        BorderColor::all(LINE),
        ring(focused, 3.5, 4.0),
    ))
    .with_children(|b| icon(b, &icons.gear, 24.0, DIM));
}

/// An engine's settings: the player's own for it, each a switch, and Done.
fn engine_settings_box(root: &mut Spawner, view: &EngineSettingsView, fonts: &Fonts) {
    modal_box(root, 680.0, |b| {
        b.spawn(Node { flex_direction: FlexDirection::Column, row_gap: px(6), padding: UiRect::new(px(18), px(18), px(14), px(14)), ..default() }).with_children(|words| {
            words.spawn((Text::new("Engine settings"), fonts.display(28.0, 800), TextColor(Color::WHITE)));
            words.spawn((Text::new(format!("{}: yours, on every course", view.name)), fonts.body(19.0, 500), TextColor(DIM)));
        });
        for (i, (label, help, value)) in view.toggles.iter().enumerate() {
            let on = view.row == i;
            b.spawn((
                Hit::Toggle(i),
                Node {
                    width: percent(100),
                    padding: UiRect::axes(px(18), px(14)),
                    column_gap: px(18),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::SpaceBetween,
                    border_radius: BorderRadius::all(px(12)),
                    ..default()
                },
                BackgroundColor(if on { SELECTED } else { CARD }),
            ))
            .with_children(|row| {
                row.spawn(Node { flex_direction: FlexDirection::Column, row_gap: px(4), flex_shrink: 1.0, ..default() }).with_children(|words| {
                    words.spawn((Text::new(label.clone()), fonts.display(22.0, 700), TextColor(Color::WHITE)));
                    words.spawn((Text::new(help.clone()), fonts.body(17.0, 400), TextColor(if on { Color::WHITE } else { DIM })));
                });
                // (the switch: filled teal when on)
                row.spawn((
                    Node {
                        width: px(96),
                        height: px(44),
                        flex_shrink: 0.0,
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(px(22)),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    BackgroundColor(if *value { TEAL } else { Color::NONE }),
                    BorderColor::all(if *value { TEAL } else { LINE }),
                ))
                .with_children(|s| {
                    s.spawn((Text::new(if *value { "On" } else { "Off" }), fonts.display(20.0, 700), TextColor(if *value { INK } else { DIM })));
                });
            });
        }
        b.spawn(Node { padding: UiRect::all(px(18)), justify_content: JustifyContent::FlexEnd, ..default() }).with_children(|buttons| {
            let on = view.row == view.toggles.len();
            buttons
                .spawn((
                    Hit::SettingsDone,
                    Node { padding: UiRect::axes(px(30), px(12)), border_radius: BorderRadius::all(px(99)), border: UiRect::all(px(1)), ..default() },
                    BackgroundColor(if on { TEAL } else { Color::NONE }),
                    BorderColor::all(if on { TEAL } else { LINE }),
                ))
                .with_children(|b| {
                    b.spawn((Text::new("Done"), fonts.display(22.0, 700), TextColor(if on { INK } else { DIM })));
                });
        });
    });
}

/// A setting's row in the course settings: its label and line, and its value between a smaller and a bigger button
/// (`hit` names them), highlighted when `on`.
fn stepper_row(b: &mut Spawner, on: bool, (label, help, value): (&str, &str, &str), (down, up): (bool, bool), hit: impl Fn(bool) -> Hit, fonts: &Fonts) {
    b.spawn((
        Node {
            width: percent(100),
            padding: UiRect::axes(px(18), px(14)),
            column_gap: px(18),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            border_radius: BorderRadius::all(px(12)),
            ..default()
        },
        BackgroundColor(if on { SELECTED } else { CARD }),
    ))
    .with_children(|row| {
        row.spawn(Node { flex_direction: FlexDirection::Column, row_gap: px(4), flex_shrink: 1.0, ..default() }).with_children(|words| {
            words.spawn((Text::new(label), fonts.display(22.0, 700), TextColor(Color::WHITE)));
            words.spawn((Text::new(help), fonts.body(17.0, 400), TextColor(if on { Color::WHITE } else { DIM })));
        });
        row.spawn(Node { column_gap: px(12), align_items: AlignItems::Center, flex_shrink: 0.0, ..default() }).with_children(|stepper| {
            step_button(stepper, hit(false), false, down, fonts);
            stepper.spawn(Node { width: px(96), justify_content: JustifyContent::Center, ..default() }).with_children(|v| {
                v.spawn((Text::new(value), fonts.display(28.0, 800), TextColor(TEAL)));
            });
            step_button(stepper, hit(true), true, up, fonts);
        });
    });
}

/// A stepper's round button: a step bigger ("+") or smaller ("−"); greyed at its end.
fn step_button(parent: &mut Spawner, hit: Hit, bigger: bool, can: bool, fonts: &Fonts) {
    parent
        .spawn((
            hit,
            Node {
                width: px(54),
                height: px(54),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(27)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::NONE),
            BorderColor::all(if can { TEAL } else { LINE }),
        ))
        .with_children(|b| {
            b.spawn((Text::new(if bigger { "+" } else { "\u{2212}" }), fonts.display(30.0, 700), TextColor(if can { Color::WHITE } else { OFF })));
        });
}

/// A round button on a card: play (filled) or the pencil.
fn round_button(parent: &mut Spawner, hit: Hit, image: &Handle<Image>, filled: bool, focused: bool) {
    parent
        .spawn((
            hit,
            Node {
                width: px(54),
                height: px(54),
                flex_shrink: 0.0,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(27)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                // (a play triangle looks centred a little right of the middle)
                padding: UiRect::left(px(if filled { 3.0 } else { 0.0 })),
                ..default()
            },
            BackgroundColor(if filled { TEAL } else { Color::NONE }),
            BorderColor::all(if filled { TEAL } else { LINE }),
            ring(focused, 3.5, 4.0),
        ))
        .with_children(|b| icon(b, image, if filled { 25.0 } else { 24.0 }, if filled { INK } else { DIM }));
}

/// A pill over a card's picture.
fn tag(parent: &mut Spawner, fonts: &Fonts, side: (Option<f32>, Option<f32>), label: &str, fg: Color, bg: Color) {
    parent
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(12),
                left: side.0.map_or(Val::Auto, px),
                right: side.1.map_or(Val::Auto, px),
                padding: UiRect::axes(px(11), px(5)),
                column_gap: px(7),
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(px(99)),
                ..default()
            },
            BackgroundColor(bg),
        ))
        .with_children(|t| {
            t.spawn((Text::new(label), fonts.body(15.5, 600), TextColor(fg)));
        });
}

fn card(row: &mut Spawner, i: usize, item: &Item, focus: Option<Focus>, icons: &Icons, fonts: &Fonts) {
    let base = Node { width: px(CARD_W), height: px(CARD_H), flex_shrink: 0.0, flex_direction: FlexDirection::Column, border_radius: BorderRadius::all(px(18)), ..default() };
    match item {
        Item::New(mode) => {
            row.spawn((
                Hit::Item(i),
                Node { border: UiRect::all(px(3)), align_items: AlignItems::Center, justify_content: JustifyContent::Center, row_gap: px(14), ..base },
                BorderColor::all(LINE),
                ring(focus == Some(Focus::Item(i, Part::Main)), 3.5, 5.0),
            ))
            .with_children(|c| {
                c.spawn((
                    Node {
                        width: px(80),
                        height: px(80),
                        border: UiRect::all(px(3.5)),
                        border_radius: BorderRadius::all(px(40)),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    BorderColor::all(TEAL),
                ))
                .with_children(|plus| {
                    plus.spawn((Text::new("+"), fonts.body(50.0, 300), TextColor(TEAL)));
                });
                c.spawn((Text::new(format!("New {}", park::course_name(*mode).to_lowercase())), fonts.display(24.0, 700), TextColor(TEAL)));
            });
        }
        Item::Park(view) => {
            let here = view.here == Here::Found;
            row.spawn((
                Hit::Item(i),
                Node { border: UiRect::all(px(1)), overflow: Overflow::clip(), ..base },
                BackgroundColor(CARD),
                BorderColor::all(LINE),
                // The open park is ringed.
                if view.current { Outline { width: px(5), offset: px(0), color: TEAL } } else { ring(false, 0.0, 0.0) },
            ))
            .with_children(|c| {
                // Its picture (dimmed where it has to be put down), its badge, and its menu.
                // (rounded as the card is: a clip is square)
                let corners = BorderRadius::top(px(17));
                c.spawn((Node { width: percent(100), height: px(THUMB_H), flex_shrink: 0.0, border_radius: corners, align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() }, BackgroundColor(THUMB_BG))).with_children(|top| {
                    match &view.thumb {
                        Some(thumb) if !view.empty => {
                            top.spawn((
                                ImageNode { color: Color::WHITE.with_alpha(if here { 1.0 } else { 0.55 }), ..ImageNode::new(thumb.clone()) },
                                Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), border_radius: corners, ..default() },
                            ));
                        }
                        _ => {
                            top.spawn((Text::new("Empty"), fonts.display(20.0, 600), TextColor(LINE)));
                        }
                    }
                    // (whether it is here, the divider says; the engine it plays on, whichever is picked)
                    if let Some(engine) = view.engine {
                        tag(top, fonts, (Some(12.0), None), engine, Color::WHITE, Color::srgba(0.0, 0.0, 0.0, 0.6));
                    }
                    if view.current {
                        tag(top, fonts, (None, Some(12.0)), "Skating", INK, TEAL);
                    }
                    top.spawn((
                        Hit::More(i),
                        Node {
                            position_type: PositionType::Absolute,
                            right: px(14),
                            bottom: px(14),
                            width: px(40),
                            height: px(40),
                            border_radius: BorderRadius::all(px(10)),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
                    ))
                    .with_children(|m| icon(m, &icons.more, 26.0, DIM));
                });
                // Its name across the card; under it, how big and when, and the buttons.
                c.spawn(Node { width: percent(100), flex_grow: 1.0, padding: UiRect::new(px(16), px(16), px(13), px(14)), flex_direction: FlexDirection::Column, row_gap: px(4), ..default() }).with_children(|meta| {
                    meta.spawn((Text::new(fitted(&view.name)), fonts.display(23.5, 700), TextColor(Color::WHITE), TextLayout::new_with_no_wrap()));
                    meta.spawn(Node { width: percent(100), align_items: AlignItems::Center, column_gap: px(10), ..default() }).with_children(|under| {
                        under.spawn((Text::new(view.sub.clone()), fonts.body(17.5, 400), TextColor(OFF), TextLayout::new_with_no_wrap(), Node { flex_grow: 1.0, flex_shrink: 1.0, min_width: px(0), ..default() }));
                        round_button(under, Hit::Edit(i), &icons.pencil, false, focus == Some(Focus::Item(i, Part::Edit)));
                        round_button(under, Hit::Play(i), &icons.play, true, focus == Some(Focus::Item(i, Part::Main)));
                    });
                });
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(id: &str, here: Here) -> Item {
        Item::Park(CardView { id: id.into(), name: id.into(), sub: String::new(), here, current: false, thumb: None, empty: false, engine: None })
    }

    #[test]
    fn cards_are_in_lines_of_three_and_the_ones_away_after_a_divider() {
        let items = vec![Item::New(Mode::Skate), view("a", Here::Found), view("b", Here::Found), view("c", Here::Found), view("d", Here::Away), view("e", Here::Away)];
        let l = lines(&items);
        assert_eq!(l, vec![Line::Cards(vec![0, 1, 2]), Line::Cards(vec![3]), Line::Divider("Not found in this room"), Line::Cards(vec![4, 5])]);
        // Only the new-park card.
        assert_eq!(lines(&[Item::New(Mode::Skate)]), vec![Line::Cards(vec![0])]);
        // Two lines of cards and a divider fit; scrolled to the last line, the first goes.
        assert_eq!(shown(&l, 0), 3);
        assert_eq!(scrolled(&l, 0, 3), 1);
        assert_eq!(scrolled(&l, 1, 0), 0);
    }

    #[test]
    fn the_stick_walks_the_cards_and_their_pencils() {
        let items = vec![Item::New(Mode::Skate), view("a", Here::Found), view("b", Here::Found), view("c", Here::Away)];
        let l = lines(&items);
        let go = |focus, f: fn(&mut Nav)| {
            let mut nav = Nav::default();
            f(&mut nav);
            moved(focus, &nav, &items, &l)
        };
        // Right from the new-park card: the next park's pencil, then the park.
        assert_eq!(go(Focus::Item(0, Part::Main), |n| n.right = true), Focus::Item(1, Part::Edit));
        assert_eq!(go(Focus::Item(1, Part::Edit), |n| n.right = true), Focus::Item(1, Part::Main));
        assert_eq!(go(Focus::Item(1, Part::Edit), |n| n.left = true), Focus::Item(0, Part::Main));
        // Down past the divider keeps the column it can; up from the top line is the first tab.
        assert_eq!(go(Focus::Item(2, Part::Edit), |n| n.down = true), Focus::Item(3, Part::Edit));
        assert_eq!(go(Focus::Item(1, Part::Main), |n| n.up = true), Focus::Tab(0));
        assert_eq!(go(Focus::Settings, |n| n.down = true), Focus::Settings, "the bottom");
        // Along the top: the tabs, the engine, its gear (passed over where it isn't shown: `frame`), and back.
        assert_eq!(go(Focus::Tab(0), |n| n.right = true), Focus::Tab(1));
        assert_eq!(go(Focus::Tab(1), |n| n.right = true), Focus::Engine);
        assert_eq!(go(Focus::Engine, |n| n.right = true), Focus::EngineGear);
        assert_eq!(go(Focus::EngineGear, |n| n.right = true), Focus::EngineGear, "the last along the top");
        assert_eq!(go(Focus::EngineGear, |n| n.left = true), Focus::Engine);
        // The buildAR Menu, bottom right: down from the last cards, up back to them.
        assert_eq!(go(Focus::Item(3, Part::Main), |n| n.down = true), Focus::Settings);
        assert_eq!(go(Focus::Settings, |n| n.up = true), Focus::Item(3, Part::Main));
        assert_eq!(go(Focus::Engine, |n| n.left = true), Focus::Tab(1));
        assert_eq!(go(Focus::Tab(1), |n| n.left = true), Focus::Tab(0));
        assert_eq!(go(Focus::Tab(0), |n| n.left = true), Focus::Tab(0));
        assert_eq!(go(Focus::Tab(1), |n| n.down = true), Focus::Item(0, Part::Main));
    }

    /// The parks screen as the headset draws it, saved as a picture to look at:
    /// `SK8_UI_SHOT=/path/parks.png cargo test ... a_picture_of_the_parks_screen -- --ignored`
    /// (`SK8_UI_SHOT_MENU=1` with a park's menu open).
    #[test]
    #[ignore = "needs a GPU; writes a picture"]
    fn a_picture_of_the_parks_screen() {
        use crate::xr::build::Placed;
        use crate::xr::build::anchor::Anchors;
        use crate::xr::build::parks::{AnchorRecord, Current, ParkEntry, ParkPose};
        use crate::xr::build::pieces::{Piece, PieceKind};
        use bevy::camera::RenderTarget;
        use bevy::render::view::screenshot::{Screenshot, save_to_disk};
        let Some(path) = std::env::var_os("SK8_UI_SHOT") else { return };
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .build()
                .disable::<bevy::winit::WinitPlugin>()
                .set(WindowPlugin { primary_window: None, exit_condition: bevy::window::ExitCondition::DontExit, ..default() }),
        );
        crate::fonts::embed(&mut app);
        let image = Image::new_target_texture(crate::xr::panel::WIDTH_PX, crate::xr::panel::HEIGHT_PX, TextureFormat::Rgba8UnormSrgb, None);
        let image = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        let camera = app
            .world_mut()
            .spawn((Camera2d, Camera { clear_color: ClearColorConfig::Custom(Color::srgb(0.3, 0.3, 0.3)), ..default() }, RenderTarget::Image(image.clone().into())))
            .id();
        app.insert_resource(XrPanel::drawn_by(camera));
        let piece = |id: u64, kind: PieceKind, x: f32, z: f32, yaw: f32, size: f32| Placed {
            id,
            piece: Piece::sized(kind, size),
            position: [x, 0.0, z],
            yaw,
            scale: 1.0,
            hidden: false,
            no_posts: 0,
            ends_down: [0.0; 2],
            finish: None,
            tint: None,
            path: None,
        };
        let park = |name: &str, opened: u64, uuid: &str, pieces: Vec<Placed>| ParkFile {
            version: 2,
            name: name.into(),
            created: 0,
            opened,
            pieces,
            scale: 8.0,
            park_scale: 1.0,
            anchors: vec![AnchorRecord { uuid: uuid.into(), offset: [0.0; 3], turn: 0.0, scale: 8.0, placed: 0 }],
            engine: None,
            engine_settings: None,
            mode: "skate".into(),
        };
        let now = park::now();
        let bowl = vec![piece(1, PieceKind::QuarterPipe, -4.0, 0.0, 0.0, 1.0), piece(2, PieceKind::QuarterPipe, 4.0, 0.0, std::f32::consts::PI, 1.0), piece(3, PieceKind::Rail, 0.0, 3.0, 1.57, 1.0), piece(4, PieceKind::FunBox, 0.0, -3.0, 0.0, 1.0)];
        let desk = vec![piece(1, PieceKind::Kicker, -3.0, 0.0, 0.5, 1.0), piece(2, PieceKind::Ledge, 2.0, 1.0, 0.0, 1.0), piece(3, PieceKind::Rail, 2.0, -2.0, 0.3, 1.0)];
        let spine = vec![piece(1, PieceKind::Spine, 0.0, 0.0, 0.0, 1.0), piece(2, PieceKind::Bank, 5.0, 0.0, 1.57, 1.0)];
        let mut parks = Parks::default();
        for (id, file) in [
            ("a", park("Living room bowl", now, "u1", bowl)),
            ("b", park("Desk lines", now - 86_400, "u2", desk)),
            ("c", park("Kitchen spine", now - 3 * 86_400, "u3", spine.clone())),
            ("d", park("Garage plaza", now - 15 * 86_400, "u4", spine)),
            ("e", park("Mini ramp", now - 40 * 86_400, "u5", Vec::new())),
            ("t", ParkFile { mode: "race".into(), ..park("Desk circuit", now - 2 * 86_400, "u2", vec![piece(1, PieceKind::Bank, 0.0, 0.0, 0.0, 1.0)]) }),
        ] {
            parks.list.push(ParkEntry { id: id.into(), file, edits: 0 });
        }
        parks.current = Some(Current { id: "a".into(), pose: ParkPose::default(), anchor: Some("u1".into()) });
        let mut anchors = Anchors::default();
        anchors.found_for_test(&["u1", "u2"]);
        app.insert_resource(parks).insert_resource(anchors).init_resource::<BuildMode>().init_resource::<ParksScreen>();
        {
            use crate::engines::{self, Engine};
            engines::add_engine(&mut app, Engine::Thug2, |_| Ok(()), |app| {
                app.add_systems(Update, || {});
            });
            engines::add_engine(&mut app, Engine::Sk8, |_| Ok(()), |app| {
                app.add_systems(Update, || {});
            });
        }
        // (`SK8_UI_SHOT_ENGINE_SETTINGS=list|open|course`: Mario Kart Wii picked, with settings of the player's - the
        // Tracks tab with the gear by the engine; its engine settings open; a track's details with its gear)
        let engine_settings = std::env::var("SK8_UI_SHOT_ENGINE_SETTINGS").ok();
        if engine_settings.is_some() {
            use crate::engines::{self, Engine, EngineToggle, EngineToggles};
            engines::add_engine(&mut app, Engine::Mkwii, |_| Ok(()), |app| {
                app.add_systems(Update, || {});
            });
            let toggles = vec![
                EngineToggle { key: "analog_throttle", label: "Analog throttle", help: "Pull the right trigger part way to drive slower" },
                EngineToggle { key: "drift_automatic", label: "Automatic drift", help: "Off: hop with the right grip and steer, for mini-turbos" },
            ];
            let mut registry = EngineToggles::default();
            registry.0.insert(Engine::Mkwii, (toggles, (|_: &World, key: &str| key == "analog_throttle", |_: &mut World, _: &str, _: bool| {})));
            app.insert_resource(registry);
            app.world_mut().resource_mut::<Parks>().list.iter_mut().find(|p| p.id == "t").unwrap().file.engine = Some("mkwii".into());
        }
        let menu = std::env::var_os("SK8_UI_SHOT_MENU").is_some();
        // (`SK8_UI_SHOT_ENGINES=1`: the engine dropdown open; `SK8_UI_SHOT_WARNING=1`: a park built for Skate 3, which
        // isn't in the app, opened)
        let dropdown = std::env::var_os("SK8_UI_SHOT_ENGINES").is_some();
        let warning = std::env::var_os("SK8_UI_SHOT_WARNING").is_some();
        // (`SK8_UI_SHOT_TAB=1`: the Tracks tab; `SK8_UI_SHOT_SETTINGS=1`: a park's settings open)
        let tracks = std::env::var_os("SK8_UI_SHOT_TAB").is_some() || std::env::var("SK8_UI_SHOT_ENGINE_SETTINGS").is_ok();
        let settings = std::env::var_os("SK8_UI_SHOT_SETTINGS").is_some();
        // (`SK8_UI_SHOT_SETTINGS=track`: a Mario Kart Wii track's, its engine's settings below the scale, one kept)
        let track_settings = std::env::var("SK8_UI_SHOT_SETTINGS").is_ok_and(|v| v == "track");
        if track_settings {
            use crate::engines::{CourseStepper, CourseSteppers, Engine};
            let step = |key, label, range| CourseStepper { key, label, help: "100% is the kart's own", range, default: 1.0, percent: true };
            let mut steppers = CourseSteppers::default();
            steppers.0.insert(Engine::Mkwii, vec![step("speed", "Top speed", (0.5, 2.0, 0.05)), step("acceleration", "Acceleration", (0.5, 3.0, 0.1))]);
            app.insert_resource(steppers);
            let mut parks = app.world_mut().resource_mut::<Parks>();
            let t = parks.list.iter_mut().find(|p| p.id == "t").unwrap();
            t.file.engine = Some("mkwii".into());
            t.file.engine_settings = Some(serde_json::json!({ "speed": 1.5 }));
        }
        // (`SK8_UI_SHOT_DETAILS=open|rename|mode|engine`: a course's details, as it says)
        let details = std::env::var("SK8_UI_SHOT_DETAILS").ok();
        if warning {
            app.world_mut().resource_mut::<Parks>().list[1].file.engine = Some("skate3".into());
        }
        app.insert_resource(XrUi { open: true, stack: vec![pages::PARKS], ..default() });
        app.add_systems(Startup, spawn).add_systems(
            Update,
            (
                move |world: &mut World| {
                    {
                        let mut screen = world.resource_mut::<ParksScreen>();
                        screen.focus = Focus::Item(if tracks { 0 } else { 2 }, Part::Main);
                        if tracks {
                            screen.tab = 1;
                        }
                        if menu {
                            screen.menu = Some(("b".into(), 1));
                        }
                        if dropdown && screen.engines.is_none() {
                            screen.engines = Some(1);
                        }
                        if warning {
                            screen.warning = Some(("b".into(), Then::Open(false), false));
                        }
                        if settings && screen.settings.is_none() {
                            screen.settings = Some(if track_settings { ("t".into(), SettingsRow::Engine(0)) } else { ("b".into(), SettingsRow::Size) });
                        }
                        if let Some(state) = &details
                            && screen.details.is_none()
                        {
                            screen.details = Some(course::Details::shown("b", "Desk lines", state));
                        }
                        match engine_settings.as_deref() {
                            Some("open") if screen.engine_settings.is_none() => screen.engine_settings = Some((crate::engines::Engine::Mkwii, 0)),
                            Some("course") if screen.details.is_none() => screen.details = Some(course::Details::shown("t", "Desk circuit", "open")),
                            _ => {}
                        }
                    }
                    if engine_settings.is_some() && world.resource::<Engines>().chosen() != Some(crate::engines::Engine::Mkwii) {
                        world.resource_mut::<Engines>().request(crate::engines::Engine::Mkwii);
                    }
                    frame(world, Nav::default(), &UiPointer::default());
                },
                draw,
            )
                .chain(),
        );
        app.finish();
        app.cleanup();
        for _ in 0..30 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        app.world_mut().spawn(Screenshot::image(image)).observe(save_to_disk(std::path::PathBuf::from(path)));
        for _ in 0..30 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Parks keep their engine: one built for an engine that isn't installed warns first (Cancel leaves it; "Play
    /// with current engine" plays it with the one running), one built for another installed engine switches to it.
    #[test]
    fn a_park_for_an_engine_not_installed_warns_and_one_installed_switches_to_it() {
        use crate::engines::{self, Engine, Engines};
        use crate::xr::build::anchor::Anchors;
        use crate::xr::build::parks::ParkEntry;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default())).init_asset::<Image>();
        engines::add_engine(&mut app, Engine::Sk8, |_| Ok(()), |app| {
            app.add_systems(Update, || {});
        });
        engines::add_engine(&mut app, Engine::Skate3, |_| Ok(()), |app| {
            app.add_systems(Update, || {});
        });
        engines::add_engine(&mut app, Engine::Thug2, |_| Err("needs your THUG2 files".into()), |app| {
            app.add_systems(Update, || {});
        });
        app.update();
        assert_eq!(app.world().resource::<Engines>().settled(), Some(Engine::Sk8));
        let park = |engine: Option<&str>| ParkFile {
            version: 2,
            name: "p".into(),
            created: 0,
            opened: 0,
            pieces: Vec::new(),
            scale: 8.0,
            park_scale: 1.0,
            anchors: Vec::new(),
            engine: engine.map(str::to_string),
            engine_settings: None,
            mode: "skate".into(),
        };
        let mut parks = Parks::default();
        for (id, engine) in [("t", Some("thug2")), ("k", Some("skate3")), ("old", None)] {
            parks.list.push(ParkEntry { id: id.into(), file: park(engine), edits: 0 });
        }
        app.insert_resource(parks)
            .init_resource::<Anchors>()
            .init_resource::<ParksScreen>()
            .init_resource::<XrUi>()
            .init_resource::<BuildMode>()
            .init_resource::<crate::xr::build::parks::Placing>()
            .init_resource::<crate::xr::build::PlacedPieces>()
            .insert_resource(crate::xr::rig::Rig { scale: 8.0, anchor: Some((Vec3::ZERO, 0.0)) });
        let world = app.world_mut();
        assert_eq!(park::engine_fit(world, "t"), EngineFit::Missing(Engine::Thug2));
        assert_eq!(park::engine_fit(world, "k"), EngineFit::Switch(Engine::Skate3));
        assert_eq!(park::engine_fit(world, "old"), EngineFit::Running);
        // THUG2's park: the warning, Cancel chosen first.
        assert_eq!(open_with_engine(world, "t", Then::Open(false)), Outcome::Stay);
        assert_eq!(world.resource::<ParksScreen>().warning, Some(("t".into(), Then::Open(false), false)));
        frame(world, Nav::default(), &UiPointer::default());
        assert_eq!(world.resource::<ParksScreen>().model.warning, Some(("THUG2".into(), false)), "the warning names the engine");
        frame(world, Nav { select: true, ..default() }, &UiPointer::default());
        assert_eq!(world.resource::<ParksScreen>().warning, None, "cancelled");
        assert!(world.resource::<crate::xr::build::parks::Placing>().0.is_none(), "and not opened");
        // Again, "Play with current engine": it opens with SK8 still running.
        open_with_engine(world, "t", Then::Open(false));
        frame(world, Nav { right: true, ..default() }, &UiPointer::default());
        frame(world, Nav { select: true, ..default() }, &UiPointer::default());
        assert_eq!(world.resource::<ParksScreen>().warning, None);
        app.update();
        assert_eq!(app.world().resource::<Engines>().settled(), Some(Engine::Sk8), "no switch");
        // Skate 3's park: straight to Skate 3.
        let world = app.world_mut();
        open_with_engine(world, "k", Then::Open(false));
        assert_eq!(world.resource::<ParksScreen>().warning, None);
        app.update();
        assert_eq!(app.world().resource::<Engines>().settled(), Some(Engine::Skate3));
        // The dropdown: THUG2 greyed (picking it says why), SK8 switches.
        let world = app.world_mut();
        world.resource_mut::<ParksScreen>().engines = Some(0);
        frame(world, Nav::default(), &UiPointer::default());
        let (running, open) = world.resource::<ParksScreen>().model.engine.clone().unwrap();
        let (rows, _) = open.unwrap();
        assert_eq!(running, "Skate 3");
        assert_eq!(rows.iter().map(|r| (r.name.as_str(), r.installed)).collect::<Vec<_>>(), vec![("SK8", true), ("Skate 3", true), ("THUG2", false)]);
        world.resource_mut::<ParksScreen>().engines = Some(2);
        assert!(matches!(frame(world, Nav { select: true, ..default() }, &UiPointer::default()), Outcome::Say(s) if s.contains("needs your THUG2 files")));
        world.resource_mut::<ParksScreen>().engines = Some(0);
        frame(world, Nav { select: true, ..default() }, &UiPointer::default());
        assert_eq!(world.resource::<ParksScreen>().engines, None, "closed");
        app.update();
        assert_eq!(app.world().resource::<Engines>().settled(), Some(Engine::Sk8));
    }

    #[test]
    fn a_park_from_above_shows_its_pieces_and_an_empty_one_nothing() {
        use crate::xr::build::Placed;
        use crate::xr::build::pieces::{Piece, PieceKind};
        let mut file = ParkFile { version: 2, name: "p".into(), created: 0, opened: 0, pieces: Vec::new(), scale: 8.0, park_scale: 1.0, anchors: Vec::new(), engine: None, engine_settings: None, mode: "skate".into() };
        let empty = thumbnail(&file, 64, 36);
        assert!(empty.chunks(4).all(|p| p == &empty[0..4]), "nothing on it");
        file.pieces.push(Placed { id: 1, piece: Piece::new(PieceKind::FunBox), position: [0.0; 3], yaw: 0.3, scale: 1.0, hidden: false, no_posts: 0, ends_down: [0.0; 2], finish: None, tint: None, path: None });
        let one = thumbnail(&file, 64, 36);
        let middle = ((18 * 64 + 32) * 4) as usize;
        assert_ne!(&one[middle..middle + 4], &empty[0..4], "the piece is in the middle");
        for icon in icons() {
            let covered = icon.chunks(4).filter(|p| p[3] > 128).count();
            assert!(covered > 100 && covered < (ICON * ICON) as usize / 2, "{covered}");
        }
        assert_eq!(when(100, 100 + 86_400 * 3), "3 days ago");
        assert_eq!(when(100, 100), "today");
    }
}
