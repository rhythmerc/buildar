//! The faithful port's adapter (imported-disc tier only): the stage and park into its physics, placing the skater,
//! and its rider, board and scoring out to the shared XR layer. What it knows of Skate 3 is `skate3_engine`'s
//! (`stage`, `trick`); this is the Bevy side.
use super::{CurrentPark, CurrentStage, PlacePlayer, SkaterLink};
use crate::physics::{GamePhysics, SkaterRuntime};
use bevy::prelude::*;

/// Adapter for the faithful port (imported-disc tier).
pub(crate) struct RetailLinkPlugin;

impl Plugin for RetailLinkPlugin {
    fn build(&self, app: &mut App) {
        use crate::app::SKATE3;
        app.init_resource::<crate::physics::OffboardView>()
            .init_resource::<Feed>()
            .add_systems(Update, (offer_stage, place_player, publish_link, publish_trick).chain().in_set(SKATE3))
            .add_systems(PreUpdate, publish_view.in_set(SKATE3))
            .add_systems(PreUpdate, hold.after(crate::xr::actions::HostInput).in_set(SKATE3));
    }
}

/// What the sim has of the stage (`skate3_engine::stage::StageFeed`).
#[derive(Resource, Default, Deref, DerefMut)]
struct Feed(skate3_engine::stage::StageFeed);

/// Skate 3 paused while the shared layer has the player busy (a menu or build mode): [`crate::app::Hold`]. (While
/// only `held` is set - aiming a spawn, placing - it keeps running on a neutral pad: `crate::input`'s
/// `pad_from_touch`.)
fn hold(held: Res<crate::xr::actions::PadHeld>, fixed: Res<Time<Fixed>>, mut hold: ResMut<crate::app::Hold>) {
    if hold.paused() != held.pause {
        info!("Skate 3 {}", if held.pause { "paused" } else { "resumed" });
    }
    hold.set(held.pause, &fixed);
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

/// The latest stage and park offered every frame: the sim takes them when it can (not mid-grind).
fn offer_stage(
    stage: Res<CurrentStage>,
    park: Res<CurrentPark>,
    physics: Option<ResMut<GamePhysics>>,
    skater: Option<ResMut<SkaterRuntime>>,
    mut feed: ResMut<Feed>,
) {
    let (Some(mut physics), Some(mut skater)) = (physics, skater) else {
        return;
    };
    feed.set_park(&park);
    if let Err(e) = feed.set_stage(&mut physics, &mut skater, &stage) {
        error!("{e}");
    }
}

fn place_player(mut requests: MessageReader<PlacePlayer>, skater: Option<ResMut<SkaterRuntime>>) {
    let Some(mut skater) = skater else {
        requests.clear();
        return;
    };
    for r in requests.read() {
        match skate3_engine::stage::place_player(&mut skater, r.point, r.forward) {
            Ok(()) => info!("XR teleport to {:?}", r.point),
            Err(e) => warn!("XR teleport: {e}"),
        }
    }
}

fn publish_link(physics: Option<Res<GamePhysics>>, skater: Option<Res<SkaterRuntime>>, mut link: ResMut<SkaterLink>) {
    let (Some(physics), Some(skater)) = (physics, skater) else {
        return;
    };
    **link = skate3_engine::stage::link(&physics, &skater);
}

/// The port's scoring for the corner readout (`skate3_engine::trick`).
fn publish_trick(skater: Option<Res<SkaterRuntime>>, mut hud: ResMut<crate::xr::ui::hud::TrickHud>) {
    let Some(skater) = skater else { return };
    let next = skate3_engine::trick::text(&skater);
    if **hud != next {
        **hud = next;
    }
}
