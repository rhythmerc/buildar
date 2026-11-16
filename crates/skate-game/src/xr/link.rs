//! What the XR layer shares with whichever skate engine runs: the faithful
//! port (imported disc) or the clean-room engine (public tier, `crate::clean`).
//!
//! `real_world` publishes the room and the placed pieces as [`ArWorld`] (game
//! metres, world space); spawn, build mode and the menus move the skater with
//! [`TeleportSkater`] and read where it is from [`SkaterLink`]. Each engine has
//! an adapter that applies the world, honours teleports and fills the link;
//! the faithful port's is [`RetailLinkPlugin`] here, the clean one lives in
//! `crate::clean::sim`.
use crate::engines::{BeforeSwitch, Engine, EngineSwitched};
use bevy::prelude::*;

#[cfg(feature = "retail")]
#[path = "link_retail.rs"]
mod retail;
#[cfg(feature = "retail")]
pub(crate) use retail::RetailLinkPlugin;

/// One placed piece's collision, world space.
#[derive(Clone, Default)]
pub(crate) struct PieceSurfaces {
    pub quads: Vec<[Vec3; 4]>,
    pub rail_quads: Vec<[Vec3; 4]>,
    pub edges: Vec<(Vec3, Vec3)>,
}

/// The rideable AR world in game metres. `revision` changes on every rebuild.
#[derive(Resource, Default)]
pub(crate) struct ArWorld {
    pub revision: u64,
    /// Scene planes and box faces.
    pub room_quads: Vec<[Vec3; 4]>,
    /// Room-mesh triangles not covered by a plane or box.
    pub room_triangles: Vec<[Vec3; 3]>,
    /// Where the room's level faces are opened over a sunk piece (a bowl):
    /// the piece, and the rectangle at its rim.
    pub holes: Vec<(u64, [Vec3; 4])>,
    /// Grindable furniture edges.
    pub room_edges: Vec<(Vec3, Vec3)>,
    pub pieces: Vec<PieceSurfaces>,
    /// Every grind rail: furniture edges and the pieces' rails and copings.
    pub rails: Vec<Vec<Vec3>>,
}

impl ArWorld {
    /// The real room as triangles (no pieces).
    pub(crate) fn room(&self) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        self.room_quads
            .iter()
            .flat_map(|&[a, b, c, d]| [[a, b, c], [a, c, d]])
            .chain(self.room_triangles.iter().copied())
    }

    /// Room and pieces as triangles.
    pub(crate) fn all_triangles(&self) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        self.room().chain(self.pieces.iter().flat_map(|p| {
            p.quads
                .iter()
                .chain(&p.rail_quads)
                .flat_map(|&[a, b, c, d]| [[a, b, c], [a, c, d]])
        }))
    }
}

/// Where the skater is, filled each frame by the running engine's adapter.
#[derive(Resource, Default)]
pub(crate) struct SkaterLink {
    /// Deck position and nose direction, game metres.
    pub deck: Option<(Vec3, Vec3)>,
    /// A grind refers to the current rails: collision waits until it ends.
    pub grinding: bool,
    /// How big the skater is drawn against life size (THUG2's world size draws it smaller); 0: life size.
    pub size: f32,
}

impl SkaterLink {
    /// The skater's size against life size (1 unless an engine says otherwise).
    pub(crate) fn size(&self) -> f32 {
        if self.size > 0.0 { self.size } else { 1.0 }
    }
}

/// Put the skater down at `point` (a ground point), facing along `forward`.
#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct TeleportSkater {
    pub point: Vec3,
    pub forward: Vec3,
}

pub(crate) struct LinkPlugin;

impl Plugin for LinkPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ArWorld>()
            .init_resource::<SkaterLink>()
            .init_resource::<CarriedSpot>()
            .add_message::<TeleportSkater>()
            .add_message::<EngineSwitched>()
            .init_resource::<BeforeSwitch>()
            .add_systems(PreUpdate, carry_over);
        app.world_mut().resource_mut::<BeforeSwitch>().0.push(note_spot);
    }
}

/// Where the skater was when its engine was left: the next engine puts its skater there.
#[derive(Resource, Default)]
struct CarriedSpot(Option<(Vec3, Vec3)>);

fn note_spot(world: &mut World, _left: Engine) {
    let deck = world.resource::<SkaterLink>().deck;
    // (only a spot in the room or the park: an engine whose skater was never put in it - the menu open since launch -
    // may still have it at its own level's start)
    let ar = world.resource::<ArWorld>();
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for t in ar.all_triangles() {
        for p in t {
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    let margin = Vec3::splat(2.0);
    let inside = |p: Vec3| lo.x <= hi.x && p.cmpge(lo - margin).all() && p.cmple(hi + margin).all();
    world.resource_mut::<CarriedSpot>().0 = deck.filter(|(p, _)| inside(*p));
}

/// After a switch: the link cleared for the new engine to fill, the trick readout emptied, and the skater put
/// where the last engine's was.
fn carry_over(
    mut switched: MessageReader<EngineSwitched>,
    mut spot: ResMut<CarriedSpot>,
    mut link: ResMut<SkaterLink>,
    mut hud: ResMut<super::ui::hud::TrickHud>,
    mut teleport: MessageWriter<TeleportSkater>,
) {
    for s in switched.read() {
        let Some(from) = s.from else { continue };
        info!("XR: {} -> {}: the skater carried over", from.name(), s.to.name());
        *link = SkaterLink::default();
        *hud = default();
        if let Some((deck, nose)) = spot.0.take() {
            let flat = nose.with_y(0.0);
            let forward = if flat.length_squared() > 1e-6 { flat.normalize() } else { Vec3::Z };
            teleport.write(TeleportSkater { point: deck, forward });
        }
    }
}

