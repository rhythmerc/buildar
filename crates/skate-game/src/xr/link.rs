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
pub(crate) use engine_api::skate::{ArWorld, PieceSurfaces, SkaterLink, TeleportSkater};

#[cfg(feature = "retail")]
#[path = "link_retail.rs"]
mod retail;
#[cfg(feature = "retail")]
pub(crate) use retail::RetailLinkPlugin;

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

