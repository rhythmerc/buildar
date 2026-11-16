//! A game's own HUD (Skate 3's trick display, THUG2's trick text and score) on small panels floating by a
//! controller: the left or the right one, as the AR settings say ([`HudHand`]). A panel may keep to a hand of its own
//! (THUG2's balance meter: by the left, where the steering is), and sit otherwise when it shares that hand.
//!
//! An engine draws its HUD into an image and spawns a quad showing it with [`HandHud`]; this places the running
//! engine's quad by the controller each frame, turned to the eyes, and shows it only while there's something to see
//! it with: the HUD on in the settings, no menu open, the controller tracked. A HUD's camera (the one drawing into
//! its image) runs only while it is shown.
use crate::engines::{EngineOwned, Engines};
use crate::xr::actions::{Grip, Hand};
use crate::xr::ar_settings::{ArSettings, HudHand};
use crate::xr::panel::XrPanel;
use crate::xr::rig::Rig;
use bevy::prelude::*;
use bevy_mod_xr::camera::XrCamera;
pub(crate) use skate_engine_api::{ABOVE_HAND_M, EngineHudCamera, HandHud};

/// Toward the eyes from the grip, real metres (so the controller doesn't cover it).
const TOWARD_EYES_M: f32 = 0.02;

pub(crate) struct HandHudPlugin;

impl Plugin for HandHudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, place.before(bevy::transform::TransformSystems::Propagate));
    }
}

/// Where a HUD panel goes, in the world: by the grip (`grip`, in the world, with `metre` the world's size of a real
/// metre), turned to `eye` (all of a hand's panels the same way, so they lie in one plane), its up the head's `up`.
pub(crate) fn by_hand(grip: Vec3, eye: Vec3, up: Vec3, metre: f32, hud: &HandHud, (offset, anchor): (Vec2, Vec2)) -> Transform {
    let to_eye = (eye - grip).normalize_or(Vec3::Z);
    // (its front, +Z, toward the eyes; level as the player's head is - not the room's up, which, looking steeply down
    // at a hand off to the side, comes out turned on its side - its +X the player's right)
    let rotation = Transform::IDENTITY.looking_to(-to_eye, up).rotation;
    let size = Vec2::new(hud.width_m, hud.width_m * hud.aspect);
    let middle = offset - anchor * size;
    let at = grip + rotation * Vec3::new(middle.x, middle.y, TOWARD_EYES_M) * metre;
    Transform { translation: at, rotation, scale: Vec3::splat(hud.width_m * metre) }
}

#[allow(clippy::type_complexity)]
fn place(
    settings: Res<ArSettings>,
    panel: Res<XrPanel>,
    engines: Option<Res<Engines>>,
    rig: Res<Rig>,
    grips: Query<(&Transform, &Grip), Without<HandHud>>,
    eyes: Query<&GlobalTransform, With<XrCamera>>,
    mut huds: Query<(&HandHud, Option<&EngineOwned>, &mut Transform, &mut Visibility)>,
    mut cameras: Query<&mut Camera>,
) {
    let running = engines.and_then(|e| e.current());
    // (each controller's grip in the world, the eyes, and the world's size of a real metre)
    let at = |hand: HudHand| {
        let root = rig.root_transform()?;
        let hand = match hand {
            HudHand::Left => Hand::Left,
            HudHand::Right => Hand::Right,
        };
        let (grip, _) = grips.iter().find(|(_, g)| g.0 == hand)?;
        let eye = eyes.iter().next()?;
        Some((root.transform_point(grip.translation), eye.translation(), eye.up().as_vec3(), root.scale.x))
    };
    for (hud, owner, mut transform, mut visibility) in &mut huds {
        // (another engine's HUD is that engine's to hide: it is hidden while it isn't running; its camera is stopped)
        if owner.is_some_and(|o| Some(o.0) != running) {
            if let Some(mut camera) = hud.camera.and_then(|c| cameras.get_mut(c).ok()) {
                if camera.is_active {
                    camera.is_active = false;
                }
            }
            continue;
        }
        let place = at(hud.hand.unwrap_or(settings.hud_hand));
        let shown = settings.hud && !panel.visible && place.is_some() && !hud.empty;
        if let Some((grip, eye, up, metre)) = place.filter(|_| shown) {
            *transform = by_hand(grip, eye, up, metre, hud, offset_of(hud, settings.hud_hand));
        }
        visibility.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
        if let Some(mut camera) = hud.camera.and_then(|c| cameras.get_mut(c).ok()) {
            if camera.is_active != shown {
                camera.is_active = shown;
            }
        }
    }
}

/// Where a panel sits from its grip (its offset and anchor), with the settings' controller `chosen`.
pub(crate) fn offset_of(hud: &HandHud, chosen: HudHand) -> (Vec2, Vec2) {
    match (hud.hand, chosen, hud.on_left) {
        (None, HudHand::Left, Some(place)) => place,
        _ => (hud.offset, hud.anchor),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The panel is above the grip, a little toward the eyes, facing them, and as wide as asked in real metres.
    #[test]
    fn the_panel_floats_above_the_hand_facing_the_eyes() {
        let grip = Vec3::new(0.2, 1.0, -0.4);
        let eye = Vec3::new(0.0, 1.6, 0.0);
        let hud = HandHud::above(0.2, 0.5, None);
        let t = by_hand(grip, eye, Vec3::Y, 2.0, &hud, (hud.offset, hud.anchor));
        // (above it as the player sees it: up the panel, which leans back with the look down at the hand)
        let up = t.rotation * Vec3::Y;
        assert!(((t.translation - grip).dot(up) - ABOVE_HAND_M * 2.0).abs() < 1e-4, "{t:?}");
        assert!(t.translation.y > grip.y + 0.15, "{t:?}");
        let front = t.rotation * Vec3::Z;
        assert!(front.dot((eye - grip).normalize()) > 0.99, "{front}");
        assert!((t.scale.x - 0.4).abs() < 1e-6);
        // (upright: its own up has no sideways lean)
        assert!((t.rotation * Vec3::X).y.abs() < 1e-5);
    }

    /// A panel anchored by its left edge to the right of the grip starts there and runs to the player's right.
    #[test]
    fn a_panel_anchored_by_its_left_edge_runs_to_the_right() {
        let grip = Vec3::new(0.0, 1.0, -0.5);
        let eye = Vec3::new(0.0, 1.0, 0.0);
        let hud = HandHud { offset: Vec2::new(0.07, 0.0), anchor: Vec2::new(-0.5, 0.0), ..HandHud::above(0.3, 0.25, None) };
        let t = by_hand(grip, eye, Vec3::Y, 1.0, &hud, (hud.offset, hud.anchor));
        // (the player looks along -z: their right is +x)
        let left_edge = t.translation.x - 0.15;
        assert!((left_edge - 0.07).abs() < 1e-4, "{t:?}");
        assert!((t.translation.y - grip.y).abs() < 1e-4);
    }

    /// A panel with an offset for sharing the left controller takes it only when it's by the left one.
    #[test]
    fn a_panel_moves_up_when_it_shares_the_left_controller() {
        let beside = (Vec2::new(-0.07, 0.03), Vec2::new(0.5, 0.0));
        let above = (Vec2::new(0.0, 0.135), Vec2::new(0.0, -0.5));
        let hud = HandHud { offset: beside.0, anchor: beside.1, on_left: Some(above), ..HandHud::above(0.1, 0.5, None) };
        assert_eq!(offset_of(&hud, HudHand::Right), beside);
        assert_eq!(offset_of(&hud, HudHand::Left), above);
        let kept = HandHud { hand: Some(HudHand::Left), ..hud };
        assert_eq!(offset_of(&kept, HudHand::Left), beside);
    }

    /// Looking steeply down at a hand off to the side, the panel stays level with the head: its right is the head's
    /// right, as far as it can be while it faces the eyes.
    #[test]
    fn a_panel_low_and_aside_stays_level_with_the_head() {
        let eye = Vec3::new(0.0, 1.6, 0.0);
        let grip = Vec3::new(0.35, 0.9, -0.15);
        // (the head looking down at it, not rolled: its right is level)
        let head = Transform::from_translation(eye).looking_at(grip, Vec3::Y);
        let hud = HandHud::above(0.2, 0.5, None);
        let t = by_hand(grip, eye, head.up().as_vec3(), 1.0, &hud, (hud.offset, hud.anchor));
        let right = t.rotation * Vec3::X;
        assert!(right.dot(head.right().as_vec3()) > 0.99, "{right} against the head's {}", head.right());
    }
}
