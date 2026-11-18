//! A course's details: what the menu opens on while a course is open (in place of the parks screen; its back arrow, or
//! B, goes to the parks screen). Its name - big, and renamed in place on a keyboard of the panel's own - its mode and
//! engine (each a dropdown), its picture, and play, build and delete; its settings (Player scale) in the course
//! settings dialog.
//!
//! Worked like the parks screen: the laser (the trigger presses), or the sticks (move, A presses, B goes back).
use super::{
    Fonts, Icons, ParksScreen, SettingsRow, Spawner, engine_rows, icon, ring, when, CARD, DANGER, DIM, INK, LINE, MENU,
    OFF, PANEL, SELECTED, TEAL, THUMB_BG,
};
use crate::engines::{Engine, Engines, Mode};
use crate::xr::build::anchor::Anchors;
use crate::xr::build::parks::{self as park, Here, Parks};
use crate::xr::build::BuildMode;
use crate::xr::ui::pointer::UiPointer;
use crate::xr::ui::{Nav, Outcome};
use bevy::prelude::*;

/// The longest name the keyboard types.
const NAME_MAX: usize = 32;

/// What can be chosen on the screen (without a dropdown or the keyboard up).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Spot {
    Back,
    Settings,
    Title,
    Mode,
    Engine,
    Play,
    Edit,
    Delete,
}

/// The screen's spots in rows, as the stick walks them.
const ROWS: [&[Spot]; 4] = [&[Spot::Back, Spot::Settings], &[Spot::Title], &[Spot::Mode, Spot::Engine], &[Spot::Play, Spot::Edit, Spot::Delete]];

/// A dropdown: the mode's or the engine's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Drop {
    Mode,
    Engine,
}

/// A key of the keyboard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Key {
    Char(char),
    Shift,
    Erase,
    Space,
    Cancel,
    Save,
}

/// The keyboard: its rows (each key with its width in units).
fn keyboard() -> Vec<Vec<(Key, f32)>> {
    let chars = |s: &str| s.chars().map(|c| (Key::Char(c), 1.0)).collect::<Vec<_>>();
    let mut third = vec![(Key::Shift, 1.5)];
    third.extend(chars("zxcvbnm-"));
    third.push((Key::Erase, 1.5));
    vec![chars("1234567890"), chars("qwertyuiop"), chars("asdfghjkl'"), third, vec![(Key::Cancel, 2.5), (Key::Space, 5.0), (Key::Save, 2.5)]]
}

/// The name being typed: the text, whether the next letter is a capital, and the key chosen.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Rename {
    text: String,
    shift: bool,
    key: (usize, usize),
}

/// The details open: of which course, what's chosen, and a dropdown or the keyboard if one is up.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Details {
    pub(crate) id: String,
    focus: Spot,
    dropdown: Option<(Drop, usize)>,
    rename: Option<Rename>,
}

impl Details {
    pub(crate) fn new(id: String) -> Self {
        Self { id, focus: Spot::Play, dropdown: None, rename: None }
    }

    /// For the picture test: the details as `state` says (`rename`, `mode`, `engine`; anything else: as opened).
    #[cfg(test)]
    pub(crate) fn shown(id: &str, name: &str, state: &str) -> Self {
        let mut d = Self::new(id.to_string());
        match state {
            "rename" => d.rename = Some(Rename { text: name.to_string(), shift: false, key: (2, 3) }),
            "mode" => d.dropdown = Some((Drop::Mode, 0)),
            "engine" => d.dropdown = Some((Drop::Engine, 1)),
            _ => {}
        }
        d
    }
}

/// What the laser can press on it.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum CHit {
    Spot(Spot),
    Row(usize),
    Key(usize, usize),
    /// Around an open dropdown: closes it.
    Away,
}

/// A dropdown's row as shown.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct RowView {
    label: String,
    sub: String,
    selected: bool,
    enabled: bool,
}

/// The screen as drawn.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct View {
    tab: String,
    title: String,
    mode: String,
    engine: String,
    thumb: Option<Handle<Image>>,
    empty: bool,
    here: bool,
    info: String,
    focus: Option<Spot>,
    dropdown: Option<(Drop, Vec<RowView>, usize)>,
    rename: Option<Rename>,
    /// Meta's keyboard is up (the panel shows only Cancel and Save).
    meta_keys: bool,
}

/// The course's mode's own words: its name, what it is.
fn mode_rows() -> Vec<(Mode, &'static str, &'static str)> {
    Mode::ALL.iter().map(|&m| (m, m.name(), if m == Mode::Skate { "Parks: a skater and a board" } else { "Tracks: a kart" })).collect()
}

/// The engine the course plays on: the running one if it's of the course's mode, else the one it was built for.
fn engine_of(world: &World, mode: Mode, built_for: Option<Engine>) -> Option<Engine> {
    let running = world.get_resource::<Engines>().and_then(|e| e.current()).filter(|e| e.mode() == mode);
    running.or(built_for)
}

/// The details' frame: input, then what to draw. (The settings and delete dialogs are the parks screen's.)
pub(super) fn frame(world: &mut World, nav: Nav, point: &UiPointer) -> Outcome {
    let Some(mut details) = world.resource::<ParksScreen>().details.clone() else { return Outcome::Stay };
    let Some(file) = world.resource::<Parks>().get(&details.id).map(|p| p.file.clone()) else {
        // (gone: to the parks)
        world.resource_mut::<ParksScreen>().details = None;
        return Outcome::Stay;
    };
    let mode = file.mode().unwrap_or(Mode::Skate);
    let hovered = point.px().and_then(|px| {
        let mut hits = world.query::<(&CHit, &ComputedNode, &UiGlobalTransform)>();
        hits.iter(world)
            .filter(|(_, node, tf)| node.size().x > 0.0 && node.contains_point(**tf, px))
            .min_by(|a, b| (a.1.size().x * a.1.size().y).total_cmp(&(b.1.size().x * b.1.size().y)))
            .map(|(hit, ..)| *hit)
    });
    let pressed = if point.click { hovered } else { None };
    let mut outcome = Outcome::Stay;

    if let Some(mut rename) = details.rename.clone() {
        // Meta's keyboard, where the headset has it (`xr::virtual_keyboard`): asked for each frame; what it typed taken.
        let (meta, typed) = match world.get_resource_mut::<crate::xr::virtual_keyboard::Typing>() {
            Some(mut typing) => {
                typing.wanted = Some(rename.text.clone());
                (typing.up, std::mem::take(&mut typing.typed))
            }
            None => (false, Vec::new()),
        };
        let mut saved_by_meta = false;
        for t in typed {
            match t {
                crate::xr::virtual_keyboard::Typed::Text(s) => {
                    for ch in s.chars() {
                        if rename.text.chars().count() < NAME_MAX && !ch.is_control() {
                            rename.text.push(ch);
                        }
                    }
                }
                crate::xr::virtual_keyboard::Typed::Erase => {
                    rename.text.pop();
                }
                crate::xr::virtual_keyboard::Typed::Enter => saved_by_meta = true,
            }
        }
        // The panel's keys (none shown with Meta's keyboard up).
        let keys = keyboard();
        if rename.key.0 >= keys.len() {
            rename.key = (keys.len() - 1, keys[keys.len() - 1].len() - 1);
        }
        let (mut r, mut c) = rename.key;
        if nav.up {
            r = (r + keys.len() - 1) % keys.len();
        }
        if nav.down {
            r = (r + 1) % keys.len();
        }
        if nav.up || nav.down {
            // (the key under the one it was on, by where along the row it is)
            c = column_near(&keys, rename.key, r);
        }
        if nav.left {
            c = (c + keys[r].len() - 1) % keys[r].len();
        }
        if nav.right {
            c = (c + 1) % keys[r].len();
        }
        if let Some(CHit::Key(kr, kc)) = hovered {
            (r, c) = (kr, kc);
        }
        rename.key = (r, c.min(keys[r].len() - 1));
        let press = if meta {
            // (Meta's keyboard types, and its Enter saves; the panel keeps its picture: A saves, B cancels)
            if saved_by_meta || nav.select {
                Some(Key::Save)
            } else if nav.back {
                Some(Key::Cancel)
            } else {
                None
            }
        } else {
            match pressed {
                Some(CHit::Key(kr, kc)) => keys.get(kr).and_then(|row| row.get(kc)).map(|k| k.0),
                Some(_) => None,
                None if nav.select => Some(keys[rename.key.0][rename.key.1].0),
                None if nav.back => Some(Key::Cancel),
                None => None,
            }
        };
        let mut done = false;
        match press {
            Some(Key::Char(ch)) if rename.text.chars().count() < NAME_MAX => {
                rename.text.push(if rename.shift { ch.to_ascii_uppercase() } else { ch });
                rename.shift = false;
            }
            Some(Key::Space) if rename.text.chars().count() < NAME_MAX => {
                rename.text.push(' ');
                rename.shift = true;
            }
            Some(Key::Erase) => {
                rename.text.pop();
                rename.shift = rename.text.is_empty() || rename.text.ends_with(' ');
            }
            Some(Key::Shift) => rename.shift = !rename.shift,
            Some(Key::Cancel) => done = true,
            Some(Key::Save) => {
                if park::rename(world, &details.id, &rename.text) {
                    outcome = Outcome::Say("Renamed".into());
                }
                done = true;
            }
            _ => {}
        }
        details.rename = if done { None } else { Some(rename) };
    } else if let Some((drop, row)) = details.dropdown {
        // A dropdown: up and down choose, A picks, B (or a press anywhere else) closes it.
        let count = match drop {
            Drop::Mode => mode_rows().len(),
            Drop::Engine => engine_rows(world, mode).map_or(0, |r| r.len()),
        };
        if count == 0 {
            details.dropdown = None;
        } else {
            let mut row = row.min(count - 1);
            if nav.up {
                row = (row + count - 1) % count;
            }
            if nav.down {
                row = (row + 1) % count;
            }
            if let Some(CHit::Row(k)) = hovered {
                row = k.min(count - 1);
            }
            let picked = match pressed {
                Some(CHit::Row(k)) => Some(Some(k)),
                Some(_) => Some(None),
                None if nav.select => Some(Some(row)),
                None if nav.back || nav.more => Some(None),
                None => None,
            };
            details.dropdown = match picked {
                Some(Some(k)) => {
                    outcome = pick(world, &details.id, mode, drop, k);
                    None
                }
                Some(None) => None,
                None => Some((drop, row)),
            };
        }
    } else {
        let mut focus = details.focus;
        let (mut r, mut c) = spot_at(focus);
        if nav.up {
            r = (r + ROWS.len() - 1) % ROWS.len();
            c = c.min(ROWS[r].len() - 1);
        }
        if nav.down {
            r = (r + 1) % ROWS.len();
            c = c.min(ROWS[r].len() - 1);
        }
        if nav.left {
            c = (c + ROWS[r].len() - 1) % ROWS[r].len();
        }
        if nav.right {
            c = (c + 1) % ROWS[r].len();
        }
        focus = ROWS[r][c];
        if let Some(CHit::Spot(s)) = hovered {
            focus = s;
        }
        details.focus = focus;
        let press = match pressed {
            Some(CHit::Spot(s)) => Some(s),
            Some(_) => None,
            None if nav.select => Some(focus),
            None => None,
        };
        match press {
            Some(Spot::Back) => {
                world.resource_mut::<ParksScreen>().details = None;
                return Outcome::Stay;
            }
            Some(Spot::Settings) => world.resource_mut::<ParksScreen>().settings = Some((details.id.clone(), SettingsRow::Size)),
            Some(Spot::Title) => {
                details.rename = Some(Rename { text: file.name.clone(), shift: false, key: (4, 2) });
            }
            Some(Spot::Mode) => {
                let at = mode_rows().iter().position(|(m, ..)| *m == mode).unwrap_or(0);
                details.dropdown = Some((Drop::Mode, at));
            }
            Some(Spot::Engine) => {
                let engine = engine_of(world, mode, file.engine());
                let at = engine_rows(world, mode).and_then(|rows| rows.iter().position(|r| Some(r.engine) == engine)).unwrap_or(0);
                details.dropdown = Some((Drop::Engine, at));
            }
            Some(Spot::Play) => outcome = Outcome::Close,
            Some(Spot::Edit) => {
                world.resource_mut::<BuildMode>().active = true;
                outcome = Outcome::Close;
            }
            Some(Spot::Delete) => world.resource_mut::<ParksScreen>().confirm = Some((details.id.clone(), false)),
            None if nav.back => {
                // (B: to the parks)
                world.resource_mut::<ParksScreen>().details = None;
                return Outcome::Stay;
            }
            None => {}
        }
    }
    if world.resource::<ParksScreen>().details.is_some() {
        world.resource_mut::<ParksScreen>().details = Some(details);
    }
    outcome
}

/// A dropdown's row `k` picked.
fn pick(world: &mut World, id: &str, mode: Mode, drop: Drop, k: usize) -> Outcome {
    match drop {
        Drop::Mode => {
            let Some(&(to, name, _)) = mode_rows().get(k) else { return Outcome::Stay };
            if to == mode {
                return Outcome::Stay;
            }
            park::set_mode(world, id, to);
            Outcome::Say(format!("Now a {} course ({})", name.to_lowercase(), park::courses_name(to)))
        }
        Drop::Engine => {
            let Some(row) = engine_rows(world, mode).and_then(|rows| rows.into_iter().nth(k)) else { return Outcome::Stay };
            if !row.installed {
                return Outcome::Say(format!("{}: {}", row.name, row.why));
            }
            if !row.running {
                world.resource_mut::<Engines>().request(row.engine);
                return Outcome::Say(format!("Switched to {}", row.name));
            }
            Outcome::Stay
        }
    }
}

/// Where a spot is in [`ROWS`].
fn spot_at(spot: Spot) -> (usize, usize) {
    ROWS.iter().enumerate().find_map(|(r, row)| row.iter().position(|&s| s == spot).map(|c| (r, c))).unwrap_or((3, 0))
}

/// The key of row `to` under key `from` (by where along its row each key's middle is).
fn column_near(keys: &[Vec<(Key, f32)>], from: (usize, usize), to: usize) -> usize {
    let middle = |row: &[(Key, f32)], c: usize| -> f32 {
        let total: f32 = row.iter().map(|k| k.1).sum();
        (row[..c].iter().map(|k| k.1).sum::<f32>() + row[c].1 * 0.5) / total
    };
    let at = middle(&keys[from.0], from.1.min(keys[from.0].len() - 1));
    (0..keys[to].len()).min_by(|&a, &b| (middle(&keys[to], a) - at).abs().total_cmp(&(middle(&keys[to], b) - at).abs())).unwrap_or(0)
}

/// The details as drawn (none: the parks screen shows).
pub(super) fn view(world: &World, modal: bool) -> Option<View> {
    let screen = world.resource::<ParksScreen>();
    let details = screen.details.as_ref()?;
    let parks = world.resource::<Parks>();
    let entry = parks.get(&details.id)?;
    let file = &entry.file;
    let mode = file.mode().unwrap_or(Mode::Skate);
    let engine = engine_of(world, mode, file.engine());
    let dropdown = details.dropdown.map(|(drop, row)| {
        let rows = match drop {
            Drop::Mode => mode_rows().into_iter().map(|(m, name, sub)| RowView { label: name.into(), sub: sub.into(), selected: m == mode, enabled: true }).collect(),
            Drop::Engine => engine_rows(world, mode)
                .unwrap_or_default()
                .into_iter()
                .map(|r| RowView {
                    sub: if r.installed { engine_line(r.engine).into() } else { r.why.clone() },
                    label: r.name,
                    selected: Some(r.engine) == engine,
                    enabled: r.installed,
                })
                .collect(),
        };
        (drop, rows, row)
    });
    let pieces = match file.pieces.len() {
        0 => "empty".to_string(),
        1 => "1 piece".to_string(),
        n => format!("{n} pieces"),
    };
    let now = park::now();
    Some(View {
        tab: park::courses_name(mode).to_string(),
        title: file.name.clone(),
        mode: mode.name().to_string(),
        engine: engine.map_or_else(|| "None".to_string(), |e| e.name().to_string()),
        thumb: screen.thumbs.get(&details.id).map(|(_, h)| h.clone()),
        empty: file.pieces.is_empty(),
        here: parks.here(&details.id, world.resource::<Anchors>()) == Here::Found,
        info: format!("{pieces} · opened {}", when(file.opened, now)),
        focus: (!modal && details.dropdown.is_none() && details.rename.is_none()).then_some(details.focus),
        dropdown,
        rename: details.rename.clone(),
        meta_keys: world.get_resource::<crate::xr::virtual_keyboard::Typing>().is_some_and(|t| t.up),
    })
}

/// What an engine plays on (the dropdown's second line).
fn engine_line(engine: Engine) -> &'static str {
    match engine {
        Engine::Sk8 => "Made for this app",
        Engine::Skate3 => "Your imported Skate 3 data",
        Engine::Thug2 => "Your THUG2 files",
        Engine::Mkwii => "Built from your disc",
    }
}

/// A pill-shaped button's look: filled (the main one), outlined, or danger.
#[derive(Clone, Copy, PartialEq)]
enum Tone {
    Main,
    Plain,
    Danger,
}

fn pill(parent: &mut Spawner, hit: CHit, tone: Tone, focused: bool, image: &Handle<Image>, label: &str, fonts: &Fonts) {
    let (fill, border, ink) = match tone {
        Tone::Main => (TEAL, TEAL, INK),
        Tone::Plain => (Color::NONE, LINE, DIM),
        Tone::Danger => (Color::NONE, Color::srgb(0.353, 0.204, 0.192), DANGER),
    };
    parent
        .spawn((
            hit,
            Node { height: px(64), padding: UiRect::axes(px(30), px(0)), column_gap: px(12), align_items: AlignItems::Center, border: UiRect::all(px(1)), border_radius: BorderRadius::all(px(99)), ..default() },
            BackgroundColor(fill),
            BorderColor::all(border),
            ring(focused, 3.5, 4.0),
        ))
        .with_children(|b| {
            icon(b, image, 24.0, ink);
            b.spawn((Text::new(label), fonts.display(if tone == Tone::Main { 24.0 } else { 22.0 }, if tone == Tone::Main { 800 } else { 700 }), TextColor(ink)));
        });
}

/// A dropdown's button: its small label, then the pill with the value and an arrow.
fn dropdown(parent: &mut Spawner, hit: CHit, label: &str, value: &str, open: bool, focused: bool, fonts: &Fonts, icons: &Icons) {
    parent.spawn((Text::new(label), fonts.body(14.0, 700), TextColor(OFF)));
    parent
        .spawn((
            hit,
            Node { height: px(48), padding: UiRect::new(px(20), px(16), px(0), px(0)), column_gap: px(10), align_items: AlignItems::Center, border: UiRect::all(px(1)), border_radius: BorderRadius::all(px(99)), ..default() },
            BackgroundColor(if open { SELECTED } else { CARD }),
            BorderColor::all(if open { TEAL } else { LINE }),
            ring(focused, 3.5, 4.0),
        ))
        .with_children(|b| {
            b.spawn((Text::new(value), fonts.body(19.0, 600), TextColor(Color::WHITE)));
            icon(b, &icons.chevron, 18.0, DIM);
        });
}

/// The details' panel (the dialogs over it are the parks screen's).
pub(super) fn panel(root: &mut Spawner, view: &View, fonts: &Fonts, icons: &Icons) {
    let focus = view.focus;
    root.spawn((
        Node { width: percent(100), height: percent(100), padding: UiRect::new(px(40), px(40), px(36), px(26)), flex_direction: FlexDirection::Column, row_gap: px(22), border_radius: BorderRadius::all(px(24)), ..default() },
        BackgroundColor(PANEL),
    ))
    .with_children(|panel| {
        // Back (to the parks), and the course's settings.
        panel.spawn(Node { width: percent(100), height: px(56), flex_shrink: 0.0, align_items: AlignItems::Center, column_gap: px(16), ..default() }).with_children(|bar| {
            bar.spawn((
                CHit::Spot(Spot::Back),
                Node { width: px(56), height: px(56), border: UiRect::all(px(1)), border_radius: BorderRadius::all(px(28)), align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
                BorderColor::all(LINE),
                ring(focus == Some(Spot::Back), 3.5, 4.0),
            ))
            .with_children(|b| icon(b, &icons.back, 30.0, DIM));
            bar.spawn((Text::new(super::spaced_caps(&view.tab)), fonts.body(16.0, 700), TextColor(OFF)));
            bar.spawn(Node { flex_grow: 1.0, ..default() });
            bar.spawn((
                CHit::Spot(Spot::Settings),
                Node { height: px(56), padding: UiRect::new(px(18), px(22), px(0), px(0)), column_gap: px(10), align_items: AlignItems::Center, border: UiRect::all(px(1)), border_radius: BorderRadius::all(px(16)), ..default() },
                BorderColor::all(LINE),
                ring(focus == Some(Spot::Settings), 3.5, 4.0),
            ))
            .with_children(|b| {
                icon(b, &icons.sliders, 26.0, DIM);
                b.spawn((Text::new("Settings"), fonts.body(19.0, 600), TextColor(DIM)));
            });
        });
        // The name (pressed: renamed), then its mode and engine.
        panel.spawn(Node { flex_direction: FlexDirection::Column, row_gap: px(14), flex_shrink: 0.0, ..default() }).with_children(|head| {
            match &view.rename {
                None => {
                    head.spawn((
                        CHit::Spot(Spot::Title),
                        Node { align_self: AlignSelf::FlexStart, align_items: AlignItems::Center, column_gap: px(16), border_radius: BorderRadius::all(px(10)), padding: UiRect::axes(px(4), px(0)), ..default() },
                        ring(focus == Some(Spot::Title), 3.5, 4.0),
                    ))
                    .with_children(|t| {
                        t.spawn((Text::new(view.title.clone()), fonts.display(64.0, 800), TextColor(Color::WHITE)));
                        icon(t, &icons.pencil, 30.0, OFF);
                    });
                }
                Some(rename) => {
                    head.spawn((
                        Node { height: px(78), padding: UiRect::axes(px(18), px(0)), align_items: AlignItems::Center, border: UiRect::all(px(2)), border_radius: BorderRadius::all(px(14)), ..default() },
                        BackgroundColor(CARD),
                        BorderColor::all(TEAL),
                    ))
                    .with_children(|field| {
                        field.spawn((Text::new(format!("{}|", rename.text)), fonts.display(50.0, 800), TextColor(Color::WHITE)));
                    });
                }
            }
            head.spawn(Node { align_items: AlignItems::Center, column_gap: px(12), ..default() }).with_children(|row| {
                let open = view.dropdown.as_ref().map(|d| d.0);
                dropdown(row, CHit::Spot(Spot::Mode), "MODE", &view.mode, open == Some(Drop::Mode), focus == Some(Spot::Mode), fonts, icons);
                row.spawn((Text::new("•"), fonts.body(24.0, 400), TextColor(OFF), Node { margin: UiRect::axes(px(4), px(0)), ..default() }));
                dropdown(row, CHit::Spot(Spot::Engine), "ENGINE", &view.engine, open == Some(Drop::Engine), focus == Some(Spot::Engine), fonts, icons);
            });
        });
        // Its picture - or, renaming, the keyboard.
        panel
            .spawn((
                Node { width: percent(100), flex_grow: 1.0, border: UiRect::all(px(1)), border_radius: BorderRadius::all(px(20)), overflow: Overflow::clip(), align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
                BackgroundColor(THUMB_BG),
                BorderColor::all(LINE),
            ))
            .with_children(|picture| match &view.rename {
                // (with Meta's keyboard up, its picture still: that keyboard is in the room)
                Some(rename) if !view.meta_keys => keys(picture, rename, fonts),
                _ => {
                    match &view.thumb {
                        Some(thumb) if !view.empty => {
                            // (its own shape, as tall as the box: the card's picture is THUMB)
                            picture.spawn((
                                ImageNode { color: Color::WHITE.with_alpha(if view.here { 1.0 } else { 0.55 }), ..ImageNode::new(thumb.clone()) },
                                Node { height: percent(100), aspect_ratio: Some(super::THUMB.0 as f32 / super::THUMB.1 as f32), ..default() },
                            ));
                        }
                        _ => {
                            picture.spawn((Text::new("Empty"), fonts.display(28.0, 600), TextColor(LINE)));
                        }
                    }
                    if view.here {
                        super::tag(picture, fonts, (Some(16.0), None), "In this room", INK, TEAL);
                    }
                    super::tag(picture, fonts, (None, Some(16.0)), &view.info, DIM, MENU.with_alpha(0.85));
                }
            });
        // Play, build, delete.
        panel.spawn(Node { width: percent(100), flex_shrink: 0.0, align_items: AlignItems::Center, column_gap: px(14), ..default() }).with_children(|row| {
            pill(row, CHit::Spot(Spot::Play), Tone::Main, focus == Some(Spot::Play), &icons.play, "Play", fonts);
            pill(row, CHit::Spot(Spot::Edit), Tone::Plain, focus == Some(Spot::Edit), &icons.pencil, "Edit", fonts);
            row.spawn(Node { flex_grow: 1.0, ..default() });
            pill(row, CHit::Spot(Spot::Delete), Tone::Danger, focus == Some(Spot::Delete), &icons.trash, "Delete", fonts);
        });
        // What the buttons do.
        panel
            .spawn((
                Node { width: percent(100), flex_shrink: 0.0, border: UiRect::top(px(1)), padding: UiRect::top(px(16)), column_gap: px(30), row_gap: px(12), flex_wrap: FlexWrap::Wrap, ..default() },
                BorderColor::all(LINE),
            ))
            .with_children(|legend| {
                let back = format!("back to {}", view.tab);
                let pairs: [(&str, &str); 4] = if view.rename.is_some() && view.meta_keys {
                    [("Enter / A", "save"), ("B", "cancel"), ("", ""), ("", "")]
                } else if view.rename.is_some() {
                    [("Trigger / A", "type"), ("B", "cancel"), ("Stick", "move"), ("", "")]
                } else {
                    [("Trigger / A", "choose"), ("B", back.as_str()), ("Menu", "back to playing"), ("Stick", "move")]
                };
                for (control, what) in pairs.into_iter().filter(|(c, _)| !c.is_empty()) {
                    legend.spawn(Node { column_gap: px(7), ..default() }).with_children(|l| {
                        l.spawn((Text::new(control), fonts.body(17.5, 600), TextColor(DIM)));
                        l.spawn((Text::new(what), fonts.body(17.5, 400), TextColor(OFF)));
                    });
                }
            });
    });
    // A dropdown's list, under its button, over the rest (a press elsewhere closes it).
    if let Some((drop, rows, chosen)) = &view.dropdown {
        root.spawn((CHit::Away, Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() }));
        // (under its button: the panel's edge, the label and the gap before it; the engine's after the mode's)
        let left = if *drop == Drop::Mode { 95.0 } else { 325.0 };
        root.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(left),
                top: px(36.0 + 56.0 + 22.0 + 77.0 + 14.0 + 48.0 + 10.0),
                width: px(380),
                padding: UiRect::all(px(8)),
                flex_direction: FlexDirection::Column,
                row_gap: px(2),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(16)),
                ..default()
            },
            BackgroundColor(MENU),
            BorderColor::all(LINE),
        ))
        .with_children(|list| {
            for (k, row) in rows.iter().enumerate() {
                let on = k == *chosen;
                list.spawn((
                    CHit::Row(k),
                    Node { padding: UiRect::axes(px(14), px(12)), column_gap: px(12), align_items: AlignItems::Center, justify_content: JustifyContent::SpaceBetween, border_radius: BorderRadius::all(px(10)), ..default() },
                    BackgroundColor(if on { SELECTED } else { Color::NONE }),
                ))
                .with_children(|r| {
                    r.spawn(Node { flex_direction: FlexDirection::Column, row_gap: px(3), ..default() }).with_children(|words| {
                        words.spawn((Text::new(row.label.clone()), fonts.body(20.0, 600), TextColor(if row.enabled { Color::WHITE } else { OFF })));
                        words.spawn((Text::new(row.sub.clone()), fonts.body(15.0, 400), TextColor(if on { DIM } else { OFF })));
                    });
                    if row.selected {
                        icon(r, &icons.check, 22.0, TEAL);
                    }
                });
            }
        });
    }
}

/// The keyboard, in the picture's place.
fn keys(parent: &mut Spawner, rename: &Rename, fonts: &Fonts) {
    let unit = 74.0;
    parent.spawn(Node { flex_direction: FlexDirection::Column, row_gap: px(10), align_items: AlignItems::Center, ..default() }).with_children(|board| {
        for (r, row) in keyboard().iter().enumerate() {
            board.spawn(Node { column_gap: px(8), ..default() }).with_children(|line| {
                for (c, &(key, width)) in row.iter().enumerate() {
                    let on = rename.key == (r, c);
                    let label = match key {
                        Key::Char(ch) => if rename.shift { ch.to_ascii_uppercase() } else { ch }.to_string(),
                        Key::Shift => "Shift".into(),
                        Key::Erase => "Erase".into(),
                        Key::Space => "Space".into(),
                        Key::Cancel => "Cancel".into(),
                        Key::Save => "Save".into(),
                    };
                    let save = key == Key::Save;
                    let lit = key == Key::Shift && rename.shift;
                    line.spawn((
                        CHit::Key(r, c),
                        Node {
                            width: px(unit * width + 8.0 * (width - 1.0)),
                            height: px(62),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            border: UiRect::all(px(1)),
                            border_radius: BorderRadius::all(px(12)),
                            ..default()
                        },
                        BackgroundColor(if save { TEAL } else if lit { SELECTED } else { CARD }),
                        BorderColor::all(if save { TEAL } else { LINE }),
                        ring(on, 3.0, 3.0),
                    ))
                    .with_children(|k| {
                        let size = if matches!(key, Key::Char(_)) { 26.0 } else { 19.0 };
                        k.spawn((Text::new(label), fonts.body(size, 600), TextColor(if save { INK } else { Color::WHITE })));
                    });
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stick_walks_the_keys_to_the_key_under_it() {
        let keys = keyboard();
        // (from Space, the bottom row's middle, up: a key in the middle of the row above, 'v' or 'b'; from 'q' down: 'a')
        assert!(matches!(keys[3][column_near(&keys, (4, 1), 3)].0, Key::Char('v' | 'b')));
        assert_eq!(column_near(&keys, (1, 0), 2), 0);
        assert_eq!(keys[4][2].0, Key::Save);
    }

    #[test]
    fn every_spot_is_in_a_row() {
        for spot in [Spot::Back, Spot::Settings, Spot::Title, Spot::Mode, Spot::Engine, Spot::Play, Spot::Edit, Spot::Delete] {
            let (r, c) = spot_at(spot);
            assert_eq!(ROWS[r][c], spot);
        }
    }
}
