//! What the XR layer shares with whichever engine runs, and the skate layer over it.
//!
//! - **Every engine:** `real_world` publishes the room and the placed pieces' solid faces as the [`CurrentStage`] (the
//!   runtime's world space: the room through the rig, in the game's metres); spawn, build mode and the menus put the
//!   player down with [`PlacePlayer`] and follow it by its [`PlayerFocus`]. The shared layer doesn't wait for an
//!   engine to take a stage: each takes the latest when it can ([`engine_core::Applied`]).
//! - **Skate engines:** the [`CurrentPark`] (rails, edges, which faces are rail tubes) comes with each stage, and the
//!   running engine fills the [`SkaterLink`] (rider and board); the running skate engine's focus is the link's
//!   ([`focus_from_link`]: the rider, else the board).
//!
//! Each engine has an adapter that offers it the stage and park, places the player and fills the link: Skate 3's is
//! [`RetailLinkPlugin`] here, SK8's lives in `crate::clean::sim`, THUG2's in `crate::thug2`.
use crate::engines::{BeforeSwitch, Engine, EngineSwitched, Engines};
use bevy::prelude::*;
pub(crate) use engine_api::skate::{CurrentPark, SkaterLink};
pub(crate) use engine_api::{CurrentStage, PlacePlayer, PlayerFocus};

#[cfg(feature = "retail")]
#[path = "link_retail.rs"]
mod retail;
#[cfg(feature = "retail")]
pub(crate) use retail::RetailLinkPlugin;

#[cfg(all(test, feature = "retail", feature = "thug2"))]
#[path = "stage_parity.rs"]
mod parity;

/// Where the room's level faces are opened over a sunk piece (a bowl): the piece, and the rectangle at its rim. The
/// stage has them opened already; this is for pointing at the room through a bowl being moved (`xr::build`).
#[derive(Resource, Default)]
pub(crate) struct SunkHoles(pub Vec<(u64, [Vec3; 4])>);

/// Where the running engine's [`PlayerFocus`] is made from its [`SkaterLink`] (PostUpdate): what's placed at the
/// player comes after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct FocusSet;

pub(crate) struct LinkPlugin;

impl Plugin for LinkPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CurrentStage>()
            .init_resource::<CurrentPark>()
            .init_resource::<SunkHoles>()
            .init_resource::<SkaterLink>()
            .init_resource::<PlayerFocus>()
            .init_resource::<CarriedSpot>()
            .add_message::<PlacePlayer>()
            .add_message::<EngineSwitched>()
            .init_resource::<BeforeSwitch>()
            .add_systems(PreUpdate, carry_over)
            .add_systems(
                PostUpdate,
                (focus_from_link.in_set(FocusSet).before(bevy::transform::TransformSystems::Propagate), keep_spot_on_rescale.after(FocusSet)),
            );
        app.world_mut().resource_mut::<BeforeSwitch>().0.push(note_spot);
    }
}

/// The running skate engine's focus: its skater (the rider, else the board), as the link has it now.
fn focus_from_link(engines: Option<Res<Engines>>, link: Res<SkaterLink>, mut focus: ResMut<PlayerFocus>) {
    if let Some(running) = engines.and_then(|e| e.current()) {
        focus.set(running, link.focus());
    }
}

/// How long the world scale is left alone before the player is put back: one placement for a whole slider drag.
const RESCALE_SETTLE_S: f32 = 0.25;

/// A world-scale change in progress: the scale and root last seen, and the player's spot in the room (tracking space)
/// from before it began.
#[derive(Default)]
struct Rescale {
    seen: Option<(f32, Transform)>,
    spot: Option<(Vec3, Vec3)>,
    changed_at: f32,
}

/// The world rescaled (any menu's slider): the player is put back on the same real spot once the scale settles. The
/// spot is taken once, from the focus before the change: while a slider moves, the engine hasn't yet taken the last
/// placement (Skate 3 turns one down while another is pending), so a focus read then is the old one, in a world
/// that has since been rescaled.
fn keep_spot_on_rescale(
    rig: Res<super::rig::Rig>,
    engines: Option<Res<Engines>>,
    focus: Res<PlayerFocus>,
    time: Res<Time<Real>>,
    mut state: Local<Rescale>,
    mut place: MessageWriter<PlacePlayer>,
) {
    let Some(root) = rig.root_transform() else {
        state.seen = None;
        return;
    };
    let now = time.elapsed_secs();
    match state.seen {
        Some((scale, before)) if scale != rig.scale => {
            if state.spot.is_none() {
                let to_room = before.compute_affine().inverse();
                state.spot = engines
                    .and_then(|e| e.current())
                    .and_then(|e| focus.of(e))
                    .map(|f| (to_room.transform_point3(f.feet), to_room.transform_vector3(f.forward)));
            }
            state.changed_at = now;
        }
        _ => {
            if now - state.changed_at >= RESCALE_SETTLE_S
                && let Some((spot, forward)) = state.spot.take()
            {
                place.write(PlacePlayer::new(root.transform_point(spot), root.rotation * forward));
            }
        }
    }
    state.seen = Some((rig.scale, root));
}

/// Where the player was when its engine was left: the next engine puts its player there.
#[derive(Resource, Default)]
struct CarriedSpot(Option<(Vec3, Vec3)>);

fn note_spot(world: &mut World, left: Engine) {
    let focus = world.resource::<PlayerFocus>().of(left);
    // (only a spot in the room or the park: an engine whose player was never put in it - the menu open since launch -
    // may still have it at its own level's start)
    let stage = world.resource::<CurrentStage>();
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for t in stage.triangles() {
        for p in t {
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    let margin = Vec3::splat(2.0);
    let inside = |p: Vec3| lo.x <= hi.x && p.cmpge(lo - margin).all() && p.cmple(hi + margin).all();
    world.resource_mut::<CarriedSpot>().0 = focus.map(|f| (f.feet, f.forward)).filter(|(p, _)| inside(*p));
}

/// After a switch: the link cleared for the new engine to fill, the trick readout emptied, and the player put where
/// the last engine's was.
fn carry_over(
    mut switched: MessageReader<EngineSwitched>,
    mut spot: ResMut<CarriedSpot>,
    mut link: ResMut<SkaterLink>,
    mut focus: ResMut<PlayerFocus>,
    mut hud: ResMut<super::ui::hud::TrickHud>,
    mut place: MessageWriter<PlacePlayer>,
) {
    for s in switched.read() {
        let Some(from) = s.from else { continue };
        info!("XR: {} -> {}: the player carried over", from.name(), s.to.name());
        *link = SkaterLink::default();
        // (the new engine's focus from when it last ran is stale until its link says where it is now)
        focus.set(s.to, None);
        *hud = default();
        if let Some((feet, forward)) = spot.0.take() {
            let flat = forward.with_y(0.0);
            let forward = if flat.length_squared() > 1e-6 { flat.normalize() } else { Vec3::Z };
            place.write(PlacePlayer::new(feet, forward));
        }
    }
}
