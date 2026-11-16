//! The stock animation graphs are the sim's (`skate3_sim::graph_runtime`); this is their Bevy
//! resource (a transitional shim: the sim has no Bevy to derive `Resource` with).
use bevy::prelude::*;
#[allow(unused_imports)]
pub(crate) use skate3_sim::graph_runtime::*;

#[derive(Resource, Clone, Debug, Deref, DerefMut)]
pub(crate) struct StockGraphs(pub skate3_sim::graph_runtime::StockGraphs);

impl StockGraphs {
    pub(crate) fn load(
        root: &std::path::Path,
        manifest: &skate_data::GameAssets,
    ) -> Result<Self, String> {
        skate3_sim::graph_runtime::StockGraphs::load(root, manifest).map(Self)
    }
}
