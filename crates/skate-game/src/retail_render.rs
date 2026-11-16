//! What is left of retail shading once maps went (legacy pass 5): the character lighting and the desktop tone
//! and exposure, which pass 6 removes too. See docs/retail-renderer.md for provenance.
use bevy::{
    asset::embedded_asset,
    prelude::*,
    render::{extract_component::ExtractComponent, render_resource::ShaderType},
};

pub(crate) struct RetailRenderPlugin;
impl Plugin for RetailRenderPlugin {
    fn build(&self, app: &mut App) {
        shadow::install(app);
        exposure::install(app);
        app.add_plugins(crate::retail_character::CharacterLightingPlugin);
        embedded_asset!(app, "retail_tone.wgsl");
    }
}

#[path = "retail_exposure.rs"]
mod exposure;

#[path = "retail_shadow.rs"]
mod shadow;
pub(crate) use shadow::ShadowState;

#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default)]
pub(crate) struct RetailTone {
    pub enabled: Vec4,
}
