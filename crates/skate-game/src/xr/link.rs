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
                focus_from_link.in_set(FocusSet).before(bevy::transform::TransformSystems::Propagate),
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

/// The running engine's player, as the shared layer follows it.
pub(crate) fn running_focus(world: &World) -> Option<engine_core::Focus> {
    let running = world.get_resource::<Engines>()?.current()?;
    world.resource::<PlayerFocus>().of(running)
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
