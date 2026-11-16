//! The faithful port's adapter (imported-disc tier only): the AR world into its physics, teleports,
//! and its deck pose and scoring out to the shared XR layer.
use super::{ArWorld, SkaterLink, TeleportSkater};
use crate::physics::ground::PlacedSurfaces;
use crate::physics::{GamePhysics, SkaterRuntime};
use bevy::prelude::*;
use skate_core::math::Vector3;

/// Adapter for the faithful port (imported-disc tier).
pub(crate) struct RetailLinkPlugin;

impl Plugin for RetailLinkPlugin {
    fn build(&self, app: &mut App) {
        use crate::app::SKATE3;
        app.init_resource::<crate::physics::OffboardView>()
            .init_resource::<crate::input::ExternalPad>()
            .add_systems(Update, (apply_world, teleport, publish_pose, publish_hud).chain().in_set(SKATE3))
            .add_systems(PreUpdate, publish_view.in_set(SKATE3))
            .add_systems(PreUpdate, hold.after(crate::xr::actions::HostInput).in_set(SKATE3))
            .add_systems(
                PreUpdate,
                pad_from_touch
                    .after(crate::xr::actions::HostInput)
                    .before(crate::input::poll_controllers)
                    .run_if(bevy_mod_openxr::openxr_session_running)
                    .in_set(SKATE3),
            );
    }
}

/// Skate 3 paused while the shared layer has the player busy (a menu or build mode): [`crate::app::Hold`]. (While
/// only `held` is set - aiming a spawn, placing - it keeps running on a neutral pad: `pad_from_touch`.)
fn hold(held: Res<crate::xr::actions::PadHeld>, fixed: Res<Time<Fixed>>, mut hold: ResMut<crate::app::Hold>) {
    if hold.paused() != held.pause {
        info!("Skate 3 {}", if held.pause { "paused" } else { "resumed" });
    }
    hold.set(held.pause, &fixed);
}

/// The game's pad from the Touch controllers (`touch::touch_to_pad`), neutral while the shared layer has them.
fn pad_from_touch(
    touch: Res<crate::xr::actions::LatestTouch>,
    held: Res<crate::xr::actions::PadHeld>,
    mut pad: ResMut<crate::input::ExternalPad>,
) {
    use crate::xr::touch::{XboxState, touch_to_pad};
    let state = if held.held {
        XboxState { buttons: 0, triggers: [0, 0], left: [0, 0], right: [0, 0] }
    } else {
        touch_to_pad(&touch.0)
    };
    pad.set(state);
}

/// Off the board the stick is relative to what the player sees: their eyes,
/// not the game camera (which the headset doesn't draw).
fn publish_view(
    rig: Res<crate::xr::rig::Rig>,
    eyes: Query<&Transform, With<bevy_mod_xr::camera::XrCamera>>,
    mut view: ResMut<crate::physics::OffboardView>,
) {
    view.0 = rig.root_transform().zip(eyes.iter().next()).map(|(root, eye)| {
        let rotation = root.rotation * eye.rotation;
        // The native presentation basis: right is -X and at is -Z of the view (`presentation`).
        [-(rotation * Vec3::X), rotation * Vec3::Y, -(rotation * Vec3::Z)].map(|axis| axis.to_array())
    });
}

fn v(p: Vec3) -> Vector3 {
    Vector3::new(p.x, p.y, p.z)
}

fn apply_world(
    world: Res<ArWorld>,
    physics: Option<ResMut<GamePhysics>>,
    skater: Option<ResMut<SkaterRuntime>>,
    mut applied: Local<Option<u64>>,
) {
    let (Some(mut physics), Some(mut skater)) = (physics, skater) else {
        return;
    };
    if world.revision == 0 || *applied == Some(world.revision) || skater.is_grinding() {
        return;
    }
    *applied = Some(world.revision);
    let (quads, triangles, edges, pieces) = surfaces(&world);
    if let Err(e) = physics.set_real_surfaces(quads, triangles, edges, pieces) {
        error!("AR collision rebuild failed: {e}");
        return;
    }
    let rails = rails(&world);
    if let Err(e) = physics.set_grind_rails(&rails, &mut skater) {
        error!("AR grind rails rebuild failed: {e}");
    }
}

pub(crate) type Surfaces = (Vec<[Vector3; 4]>, Vec<[Vector3; 3]>, Vec<(Vector3, Vector3)>, Vec<PlacedSurfaces>);

/// What `set_real_surfaces` is given for `world`.
pub(crate) fn surfaces(world: &ArWorld) -> Surfaces {
    let quads = world.room_quads.iter().map(|q| q.map(v)).collect();
    let triangles = world.room_triangles.iter().map(|t| t.map(v)).collect();
    let edges = world.room_edges.iter().map(|&(a, b)| (v(a), v(b))).collect();
    let pieces = world
        .pieces
        .iter()
        .map(|p| PlacedSurfaces {
            quads: p.quads.iter().map(|q| q.map(v)).collect(),
            rail_quads: p.rail_quads.iter().map(|q| q.map(v)).collect(),
            edges: p.edges.iter().map(|&(a, b)| (v(a), v(b))).collect(),
        })
        .collect();
    (quads, triangles, edges, pieces)
}

/// What `set_grind_rails` is given for `world`.
pub(crate) fn rails(world: &ArWorld) -> Vec<Vec<Vector3>> {
    world.rails.iter().map(|r| r.iter().map(|&p| v(p)).collect()).collect()
}

fn teleport(mut requests: MessageReader<TeleportSkater>, skater: Option<ResMut<SkaterRuntime>>) {
    let Some(mut skater) = skater else {
        requests.clear();
        return;
    };
    for r in requests.read() {
        match skater.travel_to(crate::xr::spawn::spawn_matrix(r.point, r.forward)) {
            Ok(()) => info!("XR teleport to {:?}", r.point),
            Err(e) => warn!("XR teleport: {e}"),
        }
    }
}

fn publish_pose(
    physics: Option<Res<GamePhysics>>,
    skater: Option<Res<SkaterRuntime>>,
    mut link: ResMut<SkaterLink>,
) {
    let (Some(physics), Some(skater)) = (physics, skater) else {
        return;
    };
    let at = skate3_sim::Link::of(&physics, &skater);
    link.deck = Some((at.deck, at.deck_forward));
    link.grinding = at.grinding;
}

/// The port's scoring for the corner readout: the trick being composed, the
/// sequence's points and the line.
fn publish_hud(skater: Option<Res<SkaterRuntime>>, mut hud: ResMut<crate::xr::ui::hud::TrickHud>) {
    let Some(skater) = skater else { return };
    let input = skater.scoring.hud_input();
    let points = if input.sequence_score > 0 {
        if input.multiplier > 1.0 {
            format!("{} x{:.1}", input.sequence_score, input.multiplier)
        } else {
            input.sequence_score.to_string()
        }
    } else {
        String::new()
    };
    let next = crate::xr::ui::hud::TrickHud {
        name: readable_trick(&input.trick_name),
        points,
        line: if input.line_score > 0 { format!("line {}", input.line_score) } else { String::new() },
    };
    if *hud != next {
        *hud = next;
    }
}

/// The port's trick names are its string ids and score tokens (`ID_TRICK_AIR_METRICS_FRONTFLIP`,
/// `FS_BF_50_50_FSGRAB`): shown as words (`Frontflip`, `Frontside 50-50 Frontside Grab`).
fn readable_trick(raw: &str) -> String {
    const DROP: [&str; 6] = ["ID", "TRICK", "TRICKS", "AIR", "METRICS", "NAME"];
    let word = |t: &str| -> String {
        match t {
            "BS" => "Backside".into(),
            "FS" => "Frontside".into(),
            "BF" => "".into(),
            "CROOK" => "Crooked".into(),
            "BOARD" => "Boardslide".into(),
            "LIP" => "Lipslide".into(),
            "NOSE" => "Nose".into(),
            "SHUVIT" | "SHOVEIT" => "Shove-it".into(),
            "POPSHUVIT" => "Pop Shove-it".into(),
            "FSGRAB" => "Frontside Grab".into(),
            "BSGRAB" => "Backside Grab".into(),
            "FSPOPSHUVIT" => "Frontside Pop Shove-it".into(),
            "BSPOPSHUVIT" => "Backside Pop Shove-it".into(),
            "FSSHUVIT" => "Frontside Shove-it".into(),
            "BSSHUVIT" => "Backside Shove-it".into(),
            _ => {
                let mut c = t.chars();
                match c.next() {
                    Some(f) if f.is_ascii_alphabetic() => f.to_string() + &c.as_str().to_ascii_lowercase(),
                    Some(f) => f.to_string() + c.as_str(),
                    None => String::new(),
                }
            }
        }
    };
    let mut out: Vec<String> = Vec::new();
    for part in raw.split(|c: char| c == '+' || c == '|') {
        let toks: Vec<&str> = part.split(|c: char| c == '_' || c.is_whitespace()).filter(|t| !t.is_empty()).collect();
        let mut words: Vec<String> = Vec::new();
        let mut i = 0;
        while i < toks.len() {
            let t = toks[i];
            if DROP.contains(&t) {
                i += 1;
                continue;
            }
            // 50_50 and 5_O read as 50-50 and 5-0
            if i + 1 < toks.len() && t.chars().all(|c| c.is_ascii_digit()) && (toks[i + 1].chars().all(|c| c.is_ascii_digit()) || toks[i + 1] == "O") {
                let b = if toks[i + 1] == "O" { "0" } else { toks[i + 1] };
                words.push(format!("{t}-{b}"));
                i += 2;
                continue;
            }
            let w = word(t);
            if !w.is_empty() && words.last() != Some(&w) {
                words.push(w);
            }
            i += 1;
        }
        if !words.is_empty() {
            out.push(words.join(" "));
        }
    }
    out.join(" + ")
}

#[cfg(test)]
mod readable_tests {
    #[test]
    fn port_ids_read_as_words() {
        assert_eq!(super::readable_trick("ID_TRICK_AIR_METRICS_FRONTFLIP"), "Frontflip");
        assert_eq!(super::readable_trick("BS_5_O"), "Backside 5-0");
        assert_eq!(super::readable_trick("FS_BF_50_50_FSGRAB"), "Frontside 50-50 Frontside Grab");
        assert_eq!(super::readable_trick("OLLIE"), "Ollie");
    }
}
