//! Original APT HUD rendered independently of the world's resolution scale.
use crate::{apt_scene, config::Config, hud_runtime, physics::SkaterRuntime};
use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    camera::{RenderTarget, ScalingMode, visibility::RenderLayers},
    prelude::*,
    render::render_resource::{
        AsBindGroup, BlendState, Extent3d, PrimitiveTopology, RenderPipelineDescriptor, ShaderType,
        TextureDimension, TextureFormat,
    },
    shader::ShaderRef,
    sprite_render::{AlphaMode2d, Material2d, Material2dPlugin, MeshMaterial2d},
    ui_render::UiMaterialPlugin,
    window::PrimaryWindow,
};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Copy, Debug, ShaderType)]
struct ColorTransform {
    multiply: Vec4,
    add: Vec4,
}
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct HudMaterial {
    #[uniform(0)]
    color: ColorTransform,
    #[texture(1)]
    #[sampler(2)]
    atlas: Handle<Image>,
}
impl Material2d for HudMaterial {
    fn fragment_shader() -> ShaderRef {
        concat!("embedded://", env!("CARGO_CRATE_NAME"), "/hud_render.wgsl").into()
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}
/// The offscreen target already contains RGB multiplied by coverage. Applying
/// ImageNode's straight-alpha blend again suppresses the original soft glow.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct HudComposite {
    #[texture(0)]
    #[sampler(1)]
    image: Handle<Image>,
}
impl UiMaterial for HudComposite {
    fn fragment_shader() -> ShaderRef {
        concat!("embedded://", env!("CARGO_CRATE_NAME"), "/hud_composite.wgsl").into()
    }
    fn specialize(descriptor: &mut RenderPipelineDescriptor, _: UiMaterialKey<Self>) {
        if let Some(fragment) = &mut descriptor.fragment {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING);
            }
        }
    }
}
struct Slot {
    entity: Entity,
    mesh: Handle<Mesh>,
    material: Handle<HudMaterial>,
}
#[derive(Resource)]
struct Hud {
    target: Handle<Image>,
    composite: Handle<HudComposite>,
    rebind_after_resize: bool,
    /// Its target is a size of its own (the headset's: `xr_setup`), not the window's.
    fixed: bool,
    runtime: hud_runtime::Runtime,
    source: serde_json::Value,
    shapes: apt_scene::Shapes,
    textures: BTreeMap<String, Handle<Image>>,
    slots: Vec<Slot>,
    generation: u64,
    failed: bool,
}
pub(crate) struct ScoringHudPlugin;
fn embed_render_shader(app: &mut App) {
    embedded_asset!(app, "hud_render.wgsl");
}
impl Plugin for ScoringHudPlugin {
    fn build(&self, app: &mut App) {
        embed_render_shader(app);
        embedded_asset!(app, "hud_composite.wgsl");
        app.add_plugins((
            Material2dPlugin::<HudMaterial>::default(),
            UiMaterialPlugin::<HudComposite>::default(),
        ))
        .add_systems(
            PostStartup,
            setup
                .after(crate::graphics_menu::PresentationSetup)
                // A flat-screen overlay: in XR nothing shows it, and loading
                // it stalled the first frame ~4.5 s on Quest (input queued
                // that long is an ANR).
                .run_if(|config: Res<Config>| config.xr.is_none()),
        )
        // In the headset: read on a worker (it took the first frame ~4.5 s on the main thread) and shown on a panel
        // by a controller (`xr::hand_hud`).
        .add_systems(PostStartup, xr::start.run_if(|config: Res<Config>| config.xr.is_some()))
        .add_systems(Update, xr::finish.run_if(resource_exists::<xr::Loading>))
        .add_systems(
            FixedUpdate,
            (advance, multiplier_sounds)
                .after(crate::app::SimulationSet::Physics)
                .run_if(crate::graphics_menu::gameplay_active),
        )
        .add_systems(
            Update,
            (resize_target, reset, render)
                .chain()
                .after(crate::app::FrameSet::Physics),
        );
    }
}
/// Where the player's prepared HUD is (`docs/hud-installation.md`).
fn hud_root(config: &Config) -> PathBuf {
    std::env::var_os("SKATE_SCORING_HUD_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| config.asset_root.join("private/hud"))
}

/// The HUD read from its files (all but the GPU's part): its movie started, its shapes and its textures' pixels.
struct Loaded {
    runtime: hud_runtime::Runtime,
    source: serde_json::Value,
    shapes: apt_scene::Shapes,
    textures: Vec<(String, [u32; 2], Vec<u8>)>,
}

fn load(root: &std::path::Path, input: hud_runtime::Input) -> Result<Loaded, String> {
    let source: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("runtime/trickdisplay.json"))
            .map_err(|e| format!("{}: {e}", root.display()))?,
    )
    .map_err(|e| e.to_string())?;
    let runtime = hud_runtime::Runtime::load(&source, input)?;
    let shapes: apt_scene::Shapes =
        serde_json::from_value(source["shapes"].clone()).map_err(|e| e.to_string())?;
    let mut files = BTreeMap::new();
    for shape in shapes.values().flatten() {
        files.insert(
            shape.texture.rgba.clone(),
            [shape.texture.width, shape.texture.height],
        );
    }
    for font in runtime.bindings.movie.text_assets.fonts.values() {
        files.insert(font.texture.clone(), font.size);
    }
    let mut textures = Vec::new();
    for (path, size) in files {
        let bytes = std::fs::read(root.join(&path)).map_err(|e| format!("HUD {path}: {e}"))?;
        if bytes.len() != size[0] as usize * size[1] as usize * 4 {
            return Err(format!("Invalid HUD texture size {path}"));
        }
        textures.push((path, size, bytes));
    }
    Ok(Loaded { runtime, source, shapes, textures })
}

impl Loaded {
    fn into_hud(self, images: &mut Assets<Image>) -> Hud {
        let textures = self
            .textures
            .into_iter()
            .map(|(path, size, bytes)| {
                let image = Image::new(
                    Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    TextureDimension::D2,
                    bytes,
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::RENDER_WORLD,
                );
                (path, images.add(image))
            })
            .collect();
        Hud {
            target: Handle::default(),
            composite: Handle::default(),
            rebind_after_resize: false,
            fixed: false,
            runtime: self.runtime,
            source: self.source,
            shapes: self.shapes,
            textures,
            slots: Vec::new(),
            generation: 0,
            failed: false,
        }
    }
}

fn setup(
    mut commands: Commands,
    config: Res<Config>,
    skater: Res<SkaterRuntime>,
    cameras: Query<Entity, With<IsDefaultUiCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut composites: ResMut<Assets<HudComposite>>,
) {
    // The startup dependency also applies the presentation system's deferred
    // camera spawn before this query. Without it, setup silently lost the HUD.
    let Ok(output) = cameras.single() else {
        error!("Original HUD requires the presentation camera");
        return;
    };
    let root = hud_root(&config);
    let result = load(&root, skater.scoring.hud_input()).map(|loaded| loaded.into_hud(&mut images));
    match result {
        Ok(mut hud) => {
            let target = images.add(Image::new_target_texture(
                window.physical_width().max(1),
                window.physical_height().max(1),
                TextureFormat::Rgba8UnormSrgb,
                None,
            ));
            hud.target = target.clone();
            commands.spawn((
                Camera2d,
                Projection::Orthographic(OrthographicProjection {
                    scaling_mode: ScalingMode::Fixed {
                        width: 1280.,
                        height: 720.,
                    },
                    ..OrthographicProjection::default_2d()
                }),
                Camera {
                    order: -1,
                    clear_color: ClearColorConfig::Custom(Color::NONE),
                    ..default()
                },
                RenderTarget::Image(target.clone().into()),
                RenderLayers::layer(31),
                Msaa::Off,
            ));
            hud.composite = composites.add(HudComposite { image: target });
            commands.spawn((
                MaterialNode(hud.composite.clone()),
                UiTargetCamera(output),
                GlobalZIndex(1),
                Pickable::IGNORE,
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    ..default()
                },
            ));
            commands.insert_resource(hud);
            info!("Original scoring HUD loaded from {}", root.display());
        }
        Err(error) => error!(
            "Original scoring HUD could not load from {}: {error}. See docs/hud-installation.md",
            root.display()
        ),
    }
}
// Rasterize at output pixel resolution; retain the original 1280x720 APT
// coordinate space. This avoids a second enlargement of every glyph/glow.
fn resize_target(
    hud: Option<ResMut<Hud>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut composites: ResMut<Assets<HudComposite>>,
) {
    let Some(mut hud) = hud.filter(|h| !h.fixed) else {
        return;
    };
    // Bevy 0.18's UI material preparation has no dependency on GpuImage
    // preparation. Retry once on the following frame, when the replacement
    // image is available regardless of preparation order during the resize.
    if hud.rebind_after_resize {
        if let Some(material) = composites.get_mut(&hud.composite) {
            material.image = hud.target.clone();
        }
    }
    let size = Extent3d {
        width: window.physical_width().max(1),
        height: window.physical_height().max(1),
        depth_or_array_layers: 1,
    };
    hud.rebind_after_resize = resize_image(
        &mut images,
        &mut composites,
        &hud.target,
        &hud.composite,
        size,
    );
}
fn resize_image(
    images: &mut Assets<Image>,
    composites: &mut Assets<HudComposite>,
    target: &Handle<Image>,
    composite: &Handle<HudComposite>,
    size: Extent3d,
) -> bool {
    if images
        .get(target)
        .is_some_and(|image| image.texture_descriptor.size != size)
    {
        // Assets::get_mut emits Modified even without a write. Doing that
        // every frame recreates the GPU target behind the compositor's
        // cached bind group. Only invalidate the image on a real resize.
        if let Some(image) = images.get_mut(target) {
            image.resize(size);
        }
        // The UI material retains its bind group. A real target resize must
        // reprepare it so it samples the replacement GPU texture view.
        if let Some(material) = composites.get_mut(composite) {
            material.image = target.clone();
        }
        return true;
    }
    false
}
#[cfg(test)]
mod tests {
    use super::*;

    /// The HUD mid-trick as the headset draws it (its `xr::AREA`), saved as a picture to look at:
    /// `SKATE_SCORING_HUD_ROOT=<assets>/private/hud SK8_HUD_SHOT=/path/hud.png cargo test ... a_picture_of_the_hud
    /// -- --ignored` (`SK8_HUD_SHOT_FULL=1`: the whole stage).
    #[test]
    #[ignore = "needs a GPU and the player's HUD; writes a picture"]
    fn a_picture_of_the_hud() {
        use bevy::render::view::screenshot::{Screenshot, save_to_disk};
        let (Some(path), Some(root)) = (std::env::var_os("SK8_HUD_SHOT"), std::env::var_os("SKATE_SCORING_HUD_ROOT"))
        else {
            return;
        };
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .build()
                .disable::<bevy::winit::WinitPlugin>()
                .set(WindowPlugin { primary_window: None, exit_condition: bevy::window::ExitCondition::DontExit, ..default() }),
        );
        embed_render_shader(&mut app);
        app.add_plugins(Material2dPlugin::<HudMaterial>::default());
        let input = |name: &str, score: i32, multiplier: f32, new: bool| hud_runtime::Input {
            sequence_score: score,
            line_score: score * 3,
            sequence_timer: 40,
            line_time: 40.0,
            line_capacity: 100.0,
            multiplier,
            clean: false,
            sketchy: false,
            stance: [false; 4],
            trick_name: name.into(),
            trick_metrics: [
                crate::apt_vm::Value::Text(name.into()),
                crate::apt_vm::Value::Bool(false),
                crate::apt_vm::Value::Bool(false),
                crate::apt_vm::Value::Bool(false),
                crate::apt_vm::Value::Bool(new),
            ],
            context_tricks: Vec::new(),
        };
        let loaded = load(std::path::Path::new(&root), input("", 0, 1.0, false)).expect("the HUD");
        let full = std::env::var_os("SK8_HUD_SHOT_FULL").is_some();
        let [x, y, width, height] = if full { [0.0, 0.0, 1280.0, 720.0] } else { xr::AREA };
        let mut hud = loaded.into_hud(&mut app.world_mut().resource_mut::<Assets<Image>>());
        let target = app.world_mut().resource_mut::<Assets<Image>>().add(Image::new_target_texture(
            width as u32,
            height as u32,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
        hud.target = target.clone();
        hud.fixed = true;
        hud.generation = 0;
        app.world_mut().spawn((
            Camera2d,
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: ScalingMode::Fixed { width, height },
                ..OrthographicProjection::default_2d()
            }),
            Camera { clear_color: ClearColorConfig::Custom(Color::srgb(0.25, 0.3, 0.35)), ..default() },
            Transform::from_xyz(x + width / 2.0 - 640.0, 360.0 - (y + height / 2.0), 0.0),
            RenderTarget::Image(target.clone().into()),
            RenderLayers::layer(31),
            Msaa::Off,
        ));
        app.insert_resource(hud).add_systems(Update, render);
        app.finish();
        app.cleanup();
        // (a kickflip, then a modification of it, a second trick and a multiplier, as a line's frames would)
        let steps: Vec<(hud_runtime::Input, bool, bool)> = (0..150)
            .map(|frame| match frame {
                0..20 => (input("", 0, 1.0, false), false, false),
                20 => (input("Kickflip", 500, 1.0, true), true, false),
                21..60 => (input("Kickflip", 500, 1.0, false), false, false),
                60 => (input("Kickflip to Nosegrind", 1450, 2.0, false), false, true),
                61..100 => (input("Kickflip to Nosegrind", 1450, 2.0, false), false, false),
                100 => (input("Hardflip", 3250, 3.0, true), true, false),
                _ => (input("Hardflip", 3250, 3.0, false), false, false),
            })
            .collect();
        for (input, new, modified) in steps {
            app.world_mut().resource_mut::<Hud>().runtime.update(input, new, modified, false).expect("a HUD frame");
            app.update();
        }
        app.world_mut().spawn(Screenshot::image(target)).observe(save_to_disk(std::path::PathBuf::from(path)));
        for _ in 0..20 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn hud_target_changes_only_on_resize_and_refreshes_composite() {
        // Asset scheduling only: no render plugin, window or gameplay systems.
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Image>()
            .init_asset::<HudComposite>();
        let target =
            app.world_mut()
                .resource_mut::<Assets<Image>>()
                .add(Image::new_target_texture(
                    1280,
                    720,
                    TextureFormat::Rgba8UnormSrgb,
                    None,
                ));
        let composite = app
            .world_mut()
            .resource_mut::<Assets<HudComposite>>()
            .add(HudComposite {
                image: target.clone(),
            });
        app.update();
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<Image>>>()
            .clear();
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<HudComposite>>>()
            .clear();
        for (width, height, changed) in
            [(1280, 720, false), (1920, 1080, true), (1920, 1080, false)]
        {
            app.world_mut()
                .resource_scope(|world, mut images: Mut<Assets<Image>>| {
                    let mut composites = world.resource_mut::<Assets<HudComposite>>();
                    resize_image(
                        &mut images,
                        &mut composites,
                        &target,
                        &composite,
                        Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                    );
                });
            app.update();
            let images: Vec<_> = app
                .world_mut()
                .resource_mut::<Messages<AssetEvent<Image>>>()
                .drain()
                .collect();
            let materials: Vec<_> = app
                .world_mut()
                .resource_mut::<Messages<AssetEvent<HudComposite>>>()
                .drain()
                .collect();
            assert_eq!(
                images
                    .iter()
                    .any(|e| matches!(e, AssetEvent::Modified { id } if *id == target.id())),
                changed
            );
            assert_eq!(
                materials
                    .iter()
                    .any(|e| matches!(e, AssetEvent::Modified { id } if *id == composite.id())),
                changed
            );
        }
    }
}
fn reset(
    hud: Option<ResMut<Hud>>,
    skater: Res<SkaterRuntime>,
    map: Res<crate::map_transition::CurrentMap>,
) {
    let Some(mut hud) = hud else {
        return;
    };
    if hud.generation == map.generation {
        return;
    }
    match hud_runtime::Runtime::load(&hud.source, skater.scoring.hud_input()) {
        Ok(runtime) => {
            hud.runtime = runtime;
            hud.generation = map.generation;
            hud.failed = false;
        }
        Err(error) => {
            error!("Original scoring HUD reset: {error}");
            hud.failed = true;
        }
    }
}
fn advance(
    hud: Option<ResMut<Hud>>,
    skater: Res<SkaterRuntime>,
    map: Res<crate::map_transition::CurrentMap>,
) {
    let Some(mut hud) = hud else {
        return;
    };
    if hud.failed || hud.generation != map.generation {
        return;
    }
    if let Err(error) = hud.runtime.update(
        skater.scoring.hud_input(),
        skater.scoring.new_trick,
        skater.scoring.modified_trick,
        skater.scoring.close_tricks,
    ) {
        error!("Original scoring HUD stopped: {error}");
        hud.failed = true;
    }
}
fn render(
    mut commands: Commands,
    hud: Option<ResMut<Hud>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<HudMaterial>>,
) {
    let Some(mut hud) = hud else {
        return;
    };
    let draws = if hud.failed {
        Vec::new()
    } else {
        match apt_scene::draw(&hud.runtime.bindings.movie, &hud.runtime.vm, &hud.shapes) {
            Ok(draws) => draws,
            Err(error) => {
                error!("Original HUD geometry: {error}");
                hud.failed = true;
                Vec::new()
            }
        }
    };
    for (index, draw) in draws.iter().enumerate() {
        let Some(texture) = hud.textures.get(&draw.texture).cloned() else {
            continue;
        };
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            draw.vertices
                .iter()
                .map(|v| [v.position[0] - 640., 360. - v.position[1], 0.])
                .collect::<Vec<_>>(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            draw.vertices.iter().map(|v| v.uv).collect::<Vec<_>>(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_NORMAL,
            vec![[0., 0., 1.]; draw.vertices.len()],
        );
        let material = HudMaterial {
            color: ColorTransform {
                multiply: draw.multiply.into(),
                add: draw.add.into(),
            },
            atlas: texture,
        };
        if index == hud.slots.len() {
            let mesh = meshes.add(mesh);
            let material = materials.add(material);
            let entity = commands
                .spawn((
                    Mesh2d(mesh.clone()),
                    MeshMaterial2d(material.clone()),
                    Transform::from_xyz(0., 0., index as f32 * 0.01),
                    RenderLayers::layer(31),
                ))
                .id();
            hud.slots.push(Slot {
                entity,
                mesh,
                material,
            });
        } else {
            let slot = &hud.slots[index];
            if let Some(old) = meshes.get_mut(&slot.mesh) {
                *old = mesh;
            }
            if let Some(old) = materials.get_mut(&slot.material) {
                *old = material;
            }
            commands.entity(slot.entity).insert(Visibility::Visible);
        }
    }
    for slot in &hud.slots[draws.len()..] {
        commands.entity(slot.entity).insert(Visibility::Hidden);
    }
}

/// Retail plays the score multiplier's sounds from this module, not from the ScoreModule:
/// `sub_82666BC0` polls the published sequence multiplier against its own cached copy and, on
/// any change, picks a sound by exact float equality. The scoring runtime keeps the cache and
/// reports the change edge; the selection lives with the sounds.
///
/// This runs with the simulation rather than with the frame so a change cannot be missed when
/// several fixed ticks fall inside one rendered frame. It deliberately does not depend on the
/// HUD resource: the sound is retail's whether or not the display itself loaded.
fn multiplier_sounds(
    skater: Res<SkaterRuntime>,
    mut sounds: MessageWriter<crate::skate_audio::FrontEndSoundRequest>,
) {
    let Some(multiplier) = skater.scoring.multiplier_changed else {
        return;
    };
    if let Some(id) = crate::skate_audio::player::multiplier_sound_id(multiplier) {
        sounds.write(crate::skate_audio::FrontEndSoundRequest(id));
    }
}

/// The HUD in the headset: drawn into an image of its own (the part of the stage the trick display uses) and shown on
/// a panel by a controller (`xr::hand_hud`).
mod xr {
    use super::*;
    use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future};

    /// The part of the movie's 1280x720 stage shown (x, y from its top left, width, height): the trick display's
    /// corner, bottom left (its tricks, the multiplier, the score and the line's, with room above for a long line's
    /// tricks and to the right for long names). And how many pixels a stage unit is drawn at.
    pub(super) const AREA: [f32; 4] = [40.0, 360.0, 640.0, 360.0];
    const PIXELS_PER_UNIT: f32 = 1.5;
    /// The panel's width, real metres.
    const WIDTH_M: f32 = 0.2;
    /// Where the panel's bottom left is from the grip, real metres (right, up).
    const OFFSET: Vec2 = Vec2::new(-0.055, 0.02);

    #[derive(Resource)]
    pub(super) struct Loading(Task<Result<Loaded, String>>);

    pub(super) fn start(mut commands: Commands, config: Res<Config>, skater: Res<SkaterRuntime>) {
        let root = hud_root(&config);
        let input = skater.scoring.hud_input();
        commands.insert_resource(Loading(
            AsyncComputeTaskPool::get().spawn(async move { load(&root, input).map_err(|e| format!("{}: {e}", root.display())) }),
        ));
    }

    pub(super) fn finish(
        mut commands: Commands,
        mut loading: ResMut<Loading>,
        mut images: ResMut<Assets<Image>>,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        map: Res<crate::map_transition::CurrentMap>,
    ) {
        let Some(result) = block_on(future::poll_once(&mut loading.0)) else {
            return;
        };
        commands.remove_resource::<Loading>();
        let loaded = match result {
            Ok(loaded) => loaded,
            Err(error) => {
                error!("Original scoring HUD could not load from {error}. See docs/hud-installation.md");
                return;
            }
        };
        let mut hud = loaded.into_hud(&mut images);
        let [x, y, width, height] = AREA;
        let target = images.add(Image::new_target_texture(
            (width * PIXELS_PER_UNIT) as u32,
            (height * PIXELS_PER_UNIT) as u32,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
        hud.target = target.clone();
        hud.fixed = true;
        // (started for the map that's there: `reset` would start it again, on this thread)
        hud.generation = map.generation;
        let camera = commands
            .spawn((
                crate::xr::hand_hud::EngineHudCamera,
                Camera2d,
                Projection::Orthographic(OrthographicProjection {
                    scaling_mode: ScalingMode::Fixed { width, height },
                    ..OrthographicProjection::default_2d()
                }),
                Camera {
                    order: -12,
                    clear_color: ClearColorConfig::Custom(Color::NONE),
                    is_active: false,
                    ..default()
                },
                // (the stage's middle is its (640, 360): the area's middle, in its terms)
                Transform::from_xyz(x + width / 2.0 - 640.0, 360.0 - (y + height / 2.0), 0.0),
                RenderTarget::Image(target.clone().into()),
                RenderLayers::layer(31),
                Msaa::Off,
            ))
            .id();
        commands.spawn((
            crate::xr::hand_hud::HandHud {
                // (the trick display, in the area's left part, just over the controller: the area's bottom left a
                // little left of the grip and up)
                offset: OFFSET,
                anchor: Vec2::new(-0.5, -0.5),
                ..crate::xr::hand_hud::HandHud::above(WIDTH_M, height / width, Some(camera))
            },
            Mesh3d(meshes.add(Rectangle::new(1.0, height / width))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color_texture: Some(target),
                unlit: true,
                // (the target holds colour already multiplied by its coverage)
                alpha_mode: AlphaMode::Premultiplied,
                double_sided: true,
                cull_mode: None,
                ..default()
            })),
            Transform::default(),
            Visibility::Hidden,
            Name::new("Skate 3 HUD"),
        ));
        commands.insert_resource(hud);
        info!("Original scoring HUD loaded (headset)");
    }
}
