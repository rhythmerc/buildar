//! Authored test-course grind paths. Collision/visuals share these dimensions.
//! This asset owner does not claim a nearby spline is an acquired grind.
use bevy::prelude::*;

/// Shared authored geometry. The physical grind owner runs native truck queries
/// against these same spline endpoints before state selection.
#[derive(Resource)]
pub(crate) struct GrindGeometry {
    pub rails: Vec<Rail>,
    pub native_blob: Vec<u8>,
}

pub(crate) struct GrindGeometryPlugin;
impl Plugin for GrindGeometryPlugin {
    fn build(&self, app: &mut App) {
        // (the test course's: Skate 3 has no maps)
        let geometry = GrindGeometry::test_course();
        info!(
            "GRIND_GEOMETRY rails={} bytes={} native_acquisition=paired_trucks_50_50",
            geometry.rails.len(),
            geometry.native_blob.len()
        );
        for rail in &geometry.rails {
            info!(
                "GRIND_RAIL id={} name={} start={:?} end={:?}",
                rail.id, rail.name, rail.start, rail.end
            );
        }
        app.insert_resource(geometry);
    }
}

impl GrindGeometry {
    pub(crate) fn test_course() -> Self {
        Self {
            rails: rails().to_vec(),
            native_blob: spline_blob(),
        }
    }
}

// The rails, splines and grind provider are the sim's (skate3-sim).
#[allow(unused_imports)]
pub(crate) use skate3_sim::grind_world::*;
