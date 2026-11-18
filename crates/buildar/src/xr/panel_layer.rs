//! The menu panel as a compositor layer (an OpenXR quad), as Meta advises for flat UI: the compositor samples its
//! picture once, straight to the display (with its supersampling, `XR_FB_composition_layer_settings`), where a quad
//! mesh in the scene was sampled into the eye images and those again for the lenses - soft, and shimmering when far.
//!
//! **An underlay**: submitted under the scene's projection layer (above passthrough), with the scene's pixels where
//! the panel is taken out to clear (`panel_hole.wgsl` on the panel's quad, in place of its picture), so what the scene
//! draws in front of it - the lasers and their dots, Meta's keyboard, the controllers - stays over it.
//!
//! Each frame the panel shows, the UI camera's picture is copied into the layer's swapchain image (on the render
//! thread, after the frame's drawing is submitted, before the frame ends). Until a picture has gone in - and where
//! the layer can't be made - the quad draws the picture itself, as before.
use super::panel::{HEIGHT_M, HEIGHT_PX, PanelQuad, WIDTH_M, WIDTH_PX, XrPanel};
use bevy::asset::embedded_asset;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::ShaderRef;
use bevy_mod_openxr::layer_builder::{CompositionLayer, LayerProvider, SwapchainSubImage};
use bevy_mod_openxr::resources::{OxrFrameState, OxrInstance, OxrRenderLayers, OxrSwapchain, OxrSwapchainImages};
use bevy_mod_openxr::session::OxrSession;
use bevy_mod_openxr::spaces::OxrSpaceExt as _;
use bevy_mod_openxr::types::SwapchainCreateInfo;
use bevy_mod_xr::session::{XrPreDestroySession, XrRenderSystems};
use bevy_mod_xr::spaces::XrPrimaryReferenceSpace;
use openxr::sys;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct PanelLayerPlugin;

impl Plugin for PanelLayerPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "panel_hole.wgsl");
        let live = Live::default();
        app.add_plugins((MaterialPlugin::<PanelHole>::default(), ExtractResourcePlugin::<Wanted>::default()))
            .init_resource::<Wanted>()
            .insert_resource(live.clone())
            .add_systems(Startup, spawn_hole)
            .add_systems(PostUpdate, (want, punch).chain().after(super::panel::PanelSystems));
        let Some(render) = app.get_sub_app_mut(RenderApp) else { return };
        render
            .insert_resource(live)
            .init_resource::<Chain>()
            .add_systems(RenderStartup, add_layer)
            .add_systems(
                Render,
                fill.after(RenderSystems::Render).before(XrRenderSystems::PostRender).run_if(resource_exists::<OxrSession>),
            )
            .add_systems(XrPreDestroySession, drop_chain);
    }
}

/// The extension for the layer's supersampling, asked for when the session is made.
pub(crate) fn enable(exts: &mut openxr::ExtensionSet) {
    exts.fb_composition_layer_settings = true;
}

/// A picture of the panel has gone into the layer (and it's to be shown, not the quad's own picture).
#[derive(Resource, Clone, Default)]
struct Live(Arc<AtomicBool>);

/// What the render world needs of the panel each frame.
#[derive(Resource, Clone, Default, ExtractResource)]
struct Wanted {
    visible: bool,
    /// Its pose in tracking space (the primary reference space's).
    pose: Transform,
    image: Handle<Image>,
    /// The runtime takes the supersampling setting.
    settings: bool,
    /// The picture was drawn this frame (else the layer shows the last one copied: `panel::PanelRedraw`).
    fresh: bool,
}

fn want(panel: Res<XrPanel>, redraw: Res<super::panel::PanelRedraw>, instance: Option<Res<OxrInstance>>, mut wanted: ResMut<Wanted>) {
    wanted.visible = panel.visible;
    wanted.fresh = redraw.drawn;
    wanted.pose = panel.pose();
    wanted.image = panel.image.clone();
    wanted.settings = instance.is_some_and(|i| i.exts().fb_composition_layer_settings.is_some());
}

/// The hole the layer shows through: its own quad, beside the panel's (made at startup, hidden). Not the panel's
/// quad with its material swapped: a material swapped on a visible mesh raced Bevy 0.18's specialization (a frame
/// with the entity visible and its ticks gone) and panicked at boot.
#[derive(Component)]
struct HoleQuad;

fn spawn_hole(
    panel: Res<XrPanel>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut holes: ResMut<Assets<PanelHole>>,
    mut commands: Commands,
) {
    commands.spawn((
        HoleQuad,
        Mesh3d(meshes.add(Rectangle::new(WIDTH_M, HEIGHT_M))),
        MeshMaterial3d(holes.add(PanelHole { picture: panel.image.clone() })),
        Transform::default(),
        Visibility::Hidden,
    ));
}

/// Where the layer is live: the panel's own quad hidden (`XrPanel::layered`), the hole shown where it was.
#[allow(clippy::type_complexity)]
fn punch(
    live: Res<Live>,
    mut panel: ResMut<XrPanel>,
    quads: Query<&Transform, (With<PanelQuad>, Without<HoleQuad>)>,
    mut holes: Query<(&mut Transform, &mut Visibility), With<HoleQuad>>,
) {
    let layered = live.0.load(Ordering::Acquire);
    if layered && !panel.layered {
        info!("PANEL: drawn as a compositor layer");
    }
    panel.layered = layered;
    let (Ok(quad), Ok((mut tf, mut vis))) = (quads.single(), holes.single_mut()) else { return };
    let want = if layered && panel.visible { Visibility::Visible } else { Visibility::Hidden };
    vis.set_if_neq(want);
    if *tf != *quad {
        *tf = *quad;
    }
}

/// The scene taken out where the panel is, by as much as it covers (`panel_hole.wgsl`).
#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct PanelHole {
    #[texture(0)]
    #[sampler(1)]
    picture: Handle<Image>,
}

impl Material for PanelHole {
    fn fragment_shader() -> ShaderRef {
        "embedded://buildar/xr/panel_hole.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // (what's there kept by 1 - the panel's alpha, colour and alpha both: nothing added)
        let keep = BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::OneMinusSrcAlpha, operation: BlendOperation::Add };
        if let Some(target) = descriptor.fragment.as_mut().and_then(|f| f.targets.first_mut()).and_then(Option::as_mut) {
            target.blend = Some(BlendState { color: keep, alpha: keep });
        }
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// The layer's swapchain (its images as wgpu's: two array layers each, as bevy_mod_openxr makes them; the layer is
/// the first), whether making it failed, and whether this frame's picture went in.
#[derive(Resource, Default)]
struct Chain {
    chain: Option<(OxrSwapchain, OxrSwapchainImages)>,
    /// A picture has gone in since the layer was last hidden: frames with no new picture show it.
    holds_picture: bool,
    /// Each image's first layer as a view, made once (a new view each frame was a new framebuffer each frame).
    views: Vec<bevy::render::render_resource::TextureView>,
    failed: bool,
    filled: bool,
}

/// The format of the panel's picture, and of the swapchain (copied as is).
const FORMAT: bevy::render::render_resource::TextureFormat = bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb;

/// The layer among the frame's layers, under the scene's (last): above passthrough, which goes first when it starts.
fn add_layer(layers: Option<ResMut<OxrRenderLayers>>) {
    let Some(mut layers) = layers else { return };
    let at = layers.0.len().saturating_sub(1);
    layers.0.insert(at, Box::new(PanelLayer));
}

fn drop_chain(mut chain: ResMut<Chain>, live: Res<Live>) {
    chain.views.clear();
    chain.holds_picture = false;
    chain.chain = None;
    chain.filled = false;
    live.0.store(false, Ordering::Release);
}

/// The panel's picture into the layer's next image.
#[allow(clippy::too_many_arguments)]
fn fill(
    mut chain: ResMut<Chain>,
    wanted: Res<Wanted>,
    session: Res<OxrSession>,
    frame: Option<Res<OxrFrameState>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    images: Res<RenderAssets<GpuImage>>,
    live: Res<Live>,
) {
    chain.filled = false;
    if !frame.is_some_and(|f| f.should_render) || chain.failed {
        return;
    }
    if !wanted.visible {
        chain.holds_picture = false;
    }
    // (nothing new drawn: the layer shows the last picture released)
    if wanted.visible && !wanted.fresh && chain.holds_picture && chain.chain.is_some() {
        chain.filled = true;
        return;
    }
    if chain.chain.is_none() {
        let info = SwapchainCreateInfo {
            create_flags: openxr::SwapchainCreateFlags::EMPTY,
            usage_flags: openxr::SwapchainUsageFlags::COLOR_ATTACHMENT | openxr::SwapchainUsageFlags::TRANSFER_DST | openxr::SwapchainUsageFlags::SAMPLED,
            format: FORMAT,
            sample_count: 1,
            width: WIDTH_PX,
            height: HEIGHT_PX,
            face_count: 1,
            array_size: 2,
            mip_count: 1,
        };
        let made = session
            .create_swapchain(info)
            .and_then(|swapchain| Ok((swapchain.enumerate_images(device.wgpu_device(), FORMAT, UVec2::new(WIDTH_PX, HEIGHT_PX))?, swapchain)));
        match made {
            Ok((images, swapchain)) => {
                chain.views = images
                    .0
                    .iter()
                    .map(|image| {
                        image
                            .create_view(&bevy::render::render_resource::TextureViewDescriptor {
                                dimension: Some(bevy::render::render_resource::TextureViewDimension::D2),
                                base_array_layer: 0,
                                array_layer_count: Some(1),
                                ..default()
                            })
                            .into()
                    })
                    .collect();
                chain.chain = Some((swapchain, images));
            }
            Err(why) => {
                warn!("PANEL: no compositor layer ({why}): the panel is drawn in the scene");
                chain.failed = true;
                return;
            }
        }
    }
    if !wanted.visible {
        return;
    }
    let Some(picture) = images.get(&wanted.image) else { return };
    let Chain { chain: Some((swapchain, targets)), filled, views, holds_picture, .. } = &mut *chain else { return };
    let mut went_in = false;
    crate::attached(&mut || {
        let Ok(index) = swapchain.acquire_image() else { return };
        if swapchain.wait_image(openxr::Duration::INFINITE).is_err() {
            let _ = swapchain.release_image();
            return;
        }
        let target = &targets[index as usize];
        let mut encoder = device.wgpu_device().create_command_encoder(&Default::default());
        encoder.copy_texture_to_texture(
            picture.texture.as_image_copy(),
            target.as_image_copy(),
            bevy::render::render_resource::Extent3d { width: WIDTH_PX, height: HEIGHT_PX, depth_or_array_layers: 1 },
        );
        // (left as a colour attachment, as the runtime takes it back: an empty pass that keeps what's there)
        let view = &views[index as usize];
        drop(encoder.begin_render_pass(&bevy::render::render_resource::RenderPassDescriptor {
            label: Some("panel layer"),
            color_attachments: &[Some(bevy::render::render_resource::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: bevy::render::render_resource::Operations {
                    load: bevy::render::render_resource::LoadOp::Load,
                    store: bevy::render::render_resource::StoreOp::Store,
                },
            })],
            ..default()
        }));
        queue.submit([encoder.finish()]);
        went_in = swapchain.release_image().is_ok();
    });
    *filled = went_in;
    *holds_picture |= went_in;
    if went_in && !live.0.swap(true, Ordering::AcqRel) {
        info!("PANEL: the compositor layer's first picture is in");
    }
}

/// The panel's quad layer, while a picture of it went in this frame.
struct PanelLayer;

impl LayerProvider for PanelLayer {
    fn get<'a>(&'a self, world: &'a World) -> Option<Box<dyn CompositionLayer<'a> + 'a>> {
        let chain = world.get_resource::<Chain>()?;
        let wanted = world.get_resource::<Wanted>()?;
        let space = world.get_resource::<XrPrimaryReferenceSpace>()?;
        let (swapchain, _) = chain.chain.as_ref().filter(|_| chain.filled && wanted.visible)?;
        let rect = sys::Rect2Di {
            offset: sys::Offset2Di { x: 0, y: 0 },
            extent: sys::Extent2Di { width: WIDTH_PX as i32, height: HEIGHT_PX as i32 },
        };
        let (r, t) = (wanted.pose.rotation, wanted.pose.translation);
        let mut layer = Box::new(Quad {
            inner: sys::CompositionLayerQuad {
                ty: sys::CompositionLayerQuad::TYPE,
                next: std::ptr::null(),
                layer_flags: sys::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA,
                space: space.as_raw_openxr_space(),
                eye_visibility: sys::EyeVisibility::BOTH,
                sub_image: SwapchainSubImage::new().swapchain(swapchain).image_array_index(0).image_rect(rect).into_raw(),
                pose: sys::Posef {
                    orientation: sys::Quaternionf { x: r.x, y: r.y, z: r.z, w: r.w },
                    position: sys::Vector3f { x: t.x, y: t.y, z: t.z },
                },
                size: sys::Extent2Df { width: WIDTH_M, height: HEIGHT_M },
            },
            settings: sys::CompositionLayerSettingsFB {
                ty: sys::CompositionLayerSettingsFB::TYPE,
                next: std::ptr::null(),
                layer_flags: sys::CompositionLayerSettingsFlagsFB::QUALITY_SUPER_SAMPLING,
            },
            swapchain,
        });
        if wanted.settings {
            // (the box's contents stay put: the chain points into it)
            layer.inner.next = (&raw const layer.settings).cast();
        }
        Some(layer)
    }
}

/// A quad layer, its settings chained on, and the swapchain it shows.
struct Quad<'a> {
    inner: sys::CompositionLayerQuad,
    settings: sys::CompositionLayerSettingsFB,
    swapchain: &'a OxrSwapchain,
}

// SAFETY: `header` is the quad's, which starts with the base header's fields; it and what it points to (the settings,
// the swapchain) live as long as the layer.
unsafe impl<'a> CompositionLayer<'a> for Quad<'a> {
    fn swapchain(&self) -> Option<&'a OxrSwapchain> {
        Some(self.swapchain)
    }

    fn header(&self) -> &sys::CompositionLayerBaseHeader {
        // SAFETY: as above.
        unsafe { &*(&raw const self.inner).cast::<sys::CompositionLayerBaseHeader>() }
    }
}
