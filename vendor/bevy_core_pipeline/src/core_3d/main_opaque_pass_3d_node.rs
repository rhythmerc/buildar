use crate::{
    core_3d::{Opaque3d, Transmissive3d, Transparent3d},
    skybox::{SkyboxBindGroup, SkyboxPipelineId},
};
use bevy_ecs::component::Component;
use bevy_render::{
    extract_component::ExtractComponent,
    render_phase::ViewSortedRenderPhases,
    view::RetainedViewEntity,
};
use bevy_camera::{MainPassResolutionOverride, Viewport};
use bevy_ecs::{prelude::World, query::QueryItem};
use bevy_render::{
    camera::ExtractedCamera,
    diagnostic::RecordDiagnostics,
    render_graph::{NodeRunError, RenderGraphContext, ViewNode},
    render_phase::{TrackedRenderPass, ViewBinnedRenderPhases},
    render_resource::{CommandEncoderDescriptor, PipelineCache, RenderPassDescriptor, StoreOp},
    renderer::RenderContext,
    view::{ExtractedView, ViewDepthTexture, ViewTarget, ViewUniformOffset},
};
use tracing::error;
#[cfg(feature = "trace")]
use tracing::info_span;

use super::AlphaMask3d;

/// On a 3D camera: once its main pass resolves the multisampled colour, the multisampled samples are dropped, not
/// stored (`StoreOp::Discard`), saving their write-out from tile memory on a tiling GPU. Only for cameras where no
/// pass after the main pass loads the multisampled colour again (UI drawn on the camera, an MSAA writeback, a
/// transmissive pass): with this, those would read undefined samples. (Local patch: see vendor/README.md.)
#[derive(Component, Clone, Copy, Default, Debug)]
pub struct DiscardMsaaAfterMainPass;

impl ExtractComponent for DiscardMsaaAfterMainPass {
    type QueryData = &'static Self;
    type QueryFilter = ();
    type Out = Self;

    fn extract_component(item: QueryItem<'_, '_, Self::QueryData>) -> Option<Self::Out> {
        Some(*item)
    }
}

/// Whether the view's transparent phase is drawn in its main opaque pass (local patch): when it has no transmissive
/// items, nothing is drawn between the two, and one render pass saves a tiling GPU storing the colour and depth and
/// loading them again.
pub(crate) fn transparent_in_opaque_pass(world: &World, view: &RetainedViewEntity) -> bool {
    world
        .get_resource::<ViewSortedRenderPhases<Transmissive3d>>()
        .and_then(|phases| phases.get(view))
        .is_none_or(|phase| phase.items.is_empty())
}

/// A [`bevy_render::render_graph::Node`] that runs the [`Opaque3d`] and [`AlphaMask3d`]
/// [`ViewBinnedRenderPhases`]s.
#[derive(Default)]
pub struct MainOpaquePass3dNode;
impl ViewNode for MainOpaquePass3dNode {
    type ViewQuery = (
        &'static ExtractedCamera,
        &'static ExtractedView,
        &'static ViewTarget,
        &'static ViewDepthTexture,
        Option<&'static SkyboxPipelineId>,
        Option<&'static SkyboxBindGroup>,
        &'static ViewUniformOffset,
        Option<&'static MainPassResolutionOverride>,
        bevy_ecs::query::Has<DiscardMsaaAfterMainPass>,
    );

    fn run<'w>(
        &self,
        graph: &mut RenderGraphContext,
        render_context: &mut RenderContext<'w>,
        (
            camera,
            extracted_view,
            target,
            depth,
            skybox_pipeline,
            skybox_bind_group,
            view_uniform_offset,
            resolution_override,
            discard_msaa,
        ): QueryItem<'w, '_, Self::ViewQuery>,
        world: &'w World,
    ) -> Result<(), NodeRunError> {
        let (Some(opaque_phases), Some(alpha_mask_phases)) = (
            world.get_resource::<ViewBinnedRenderPhases<Opaque3d>>(),
            world.get_resource::<ViewBinnedRenderPhases<AlphaMask3d>>(),
        ) else {
            return Ok(());
        };

        let (Some(opaque_phase), Some(alpha_mask_phase)) = (
            opaque_phases.get(&extracted_view.retained_view_entity),
            alpha_mask_phases.get(&extracted_view.retained_view_entity),
        ) else {
            return Ok(());
        };

        let diagnostics = render_context.diagnostic_recorder();

        // (local patch: the transparent phase in this same pass where it can be, and then the multisampled colour,
        // resolved, dropped where the camera asks)
        let transparent_phase = transparent_in_opaque_pass(world, &extracted_view.retained_view_entity)
            .then(|| {
                world
                    .get_resource::<ViewSortedRenderPhases<Transparent3d>>()
                    .and_then(|phases| phases.get(&extracted_view.retained_view_entity))
            })
            .flatten();
        let mut color_attachment = target.get_color_attachment();
        if transparent_phase.is_some() && discard_msaa && color_attachment.resolve_target.is_some() {
            color_attachment.ops.store = StoreOp::Discard;
        }
        let color_attachments = [Some(color_attachment)];
        let depth_stencil_attachment = Some(depth.get_attachment(StoreOp::Store));

        let view_entity = graph.view_entity();
        render_context.add_command_buffer_generation_task(move |render_device| {
            #[cfg(feature = "trace")]
            let _main_opaque_pass_3d_span = info_span!("main_opaque_pass_3d").entered();

            // Command encoder setup
            let mut command_encoder =
                render_device.create_command_encoder(&CommandEncoderDescriptor {
                    label: Some("main_opaque_pass_3d_command_encoder"),
                });

            // Render pass setup
            let render_pass = command_encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("main_opaque_pass_3d"),
                color_attachments: &color_attachments,
                depth_stencil_attachment,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let mut render_pass = TrackedRenderPass::new(&render_device, render_pass);
            let pass_span = diagnostics.pass_span(&mut render_pass, "main_opaque_pass_3d");

            if let Some(viewport) =
                Viewport::from_viewport_and_override(camera.viewport.as_ref(), resolution_override)
            {
                render_pass.set_camera_viewport(&viewport);
            }

            // Opaque draws
            if !opaque_phase.is_empty() {
                #[cfg(feature = "trace")]
                let _opaque_main_pass_3d_span = info_span!("opaque_main_pass_3d").entered();
                if let Err(err) = opaque_phase.render(&mut render_pass, world, view_entity) {
                    error!("Error encountered while rendering the opaque phase {err:?}");
                }
            }

            // Alpha draws
            if !alpha_mask_phase.is_empty() {
                #[cfg(feature = "trace")]
                let _alpha_mask_main_pass_3d_span = info_span!("alpha_mask_main_pass_3d").entered();
                if let Err(err) = alpha_mask_phase.render(&mut render_pass, world, view_entity) {
                    error!("Error encountered while rendering the alpha mask phase {err:?}");
                }
            }

            // Skybox draw using a fullscreen triangle
            if let (Some(skybox_pipeline), Some(SkyboxBindGroup(skybox_bind_group))) =
                (skybox_pipeline, skybox_bind_group)
            {
                let pipeline_cache = world.resource::<PipelineCache>();
                if let Some(pipeline) = pipeline_cache.get_render_pipeline(skybox_pipeline.0) {
                    render_pass.set_render_pipeline(pipeline);
                    render_pass.set_bind_group(
                        0,
                        &skybox_bind_group.0,
                        &[view_uniform_offset.offset, skybox_bind_group.1],
                    );
                    render_pass.draw(0..3, 0..1);
                }
            }

            // Transparent draws, sorted back-to-front (local patch: here, not in a pass of their own, when there is
            // nothing transmissive between them and the opaque ones)
            if let Some(transparent_phase) = transparent_phase
                && !transparent_phase.items.is_empty()
            {
                #[cfg(feature = "trace")]
                let _transparent_main_pass_3d_span = info_span!("transparent_in_main_opaque_pass_3d").entered();
                if let Err(err) = transparent_phase.render(&mut render_pass, world, view_entity) {
                    error!("Error encountered while rendering the transparent phase {err:?}");
                }
            }

            pass_span.end(&mut render_pass);
            {
                // wgpu may perform substantial validation/encoding when the pass
                // closes. Separate that CPU work from draw-list traversal.
                #[cfg(feature = "trace")]
                let _span = info_span!("main_opaque_pass_close").entered();
                drop(render_pass);
            }
            {
                #[cfg(feature = "trace")]
                let _span = info_span!("main_opaque_encoder_finish").entered();
                command_encoder.finish()
            }
        });

        Ok(())
    }
}
