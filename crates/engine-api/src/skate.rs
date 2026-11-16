//! Skate mode's part of the contract ([`Mode::Skate`](crate::Mode::Skate)): the park and room an engine's skater
//! rides, where to put the skater, where it is, and its trick text.
//!
//! `ArWorld`'s room half (`room_quads`, `room_triangles`) isn't skate's; it moves to the core when a second mode
//! needs it (docs/buildar-proposal.md).
use bevy::prelude::*;

/// One placed piece's collision, world space.
#[derive(Clone, Default)]
pub struct PieceSurfaces {
    pub quads: Vec<[Vec3; 4]>,
    pub rail_quads: Vec<[Vec3; 4]>,
    pub edges: Vec<(Vec3, Vec3)>,
}

/// The rideable AR world in game metres. `revision` changes on every rebuild.
#[derive(Resource, Default)]
pub struct ArWorld {
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
    pub fn room(&self) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        self.room_quads
            .iter()
            .flat_map(|&[a, b, c, d]| [[a, b, c], [a, c, d]])
            .chain(self.room_triangles.iter().copied())
    }

    /// Room and pieces as triangles.
    pub fn all_triangles(&self) -> impl Iterator<Item = [Vec3; 3]> + '_ {
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
pub struct SkaterLink {
    /// Deck position and nose direction, game metres.
    pub deck: Option<(Vec3, Vec3)>,
    /// A grind refers to the current rails: collision waits until it ends.
    pub grinding: bool,
    /// How big the skater is drawn against life size (THUG2's world size draws it smaller); 0: life size.
    pub size: f32,
}

impl SkaterLink {
    /// The skater's size against life size (1 unless an engine says otherwise).
    pub fn size(&self) -> f32 {
        if self.size > 0.0 { self.size } else { 1.0 }
    }
}

/// Put the skater down at `point` (a ground point), facing along `forward`.
#[derive(Message, Clone, Copy, Debug)]
pub struct TeleportSkater {
    pub point: Vec3,
    pub forward: Vec3,
}

/// What the generic trick readout shows; an engine without a HUD of its own writes it every frame.
#[derive(Resource, Default, Clone, PartialEq)]
pub struct TrickHud {
    /// The trick in progress or just landed.
    pub name: String,
    /// Points it scored, if any.
    pub points: String,
    /// Score, combo, a new best.
    pub line: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_world_is_its_room_and_pieces_as_triangles() {
        let quad = [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 0.0, 1.0), Vec3::Z];
        let world = ArWorld {
            room_quads: vec![quad],
            room_triangles: vec![[Vec3::ZERO, Vec3::Y, Vec3::X]],
            pieces: vec![PieceSurfaces { quads: vec![quad], rail_quads: vec![quad], ..default() }],
            ..default()
        };
        assert_eq!(world.room().count(), 3);
        assert_eq!(world.all_triangles().count(), 7);
    }
}
