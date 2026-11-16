//! The original APT HUD. In the headset it is drawn into an image of its own and shown on a panel by a controller
//! (`xr::hand_hud`); the flat desktop run has none (legacy pass 4).
use crate::{apt_scene, config::Config, hud_runtime, physics::SkaterRuntime};
use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    camera::{RenderTarget, ScalingMode, visibility::RenderLayers},
    prelude::*,
    render::render_resource::{
        AsBindGroup, Extent3d, PrimitiveTopology, ShaderType, TextureDimension, TextureFormat,
    },
    shader::ShaderRef,
    sprite_render::{AlphaMode2d, Material2d, Material2dPlugin, MeshMaterial2d},
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
struct Slot {
    entity: Entity,
    mesh: Handle<Mesh>,
    material: Handle<HudMaterial>,
}
#[derive(Resource)]
struct Hud {
    runtime: hud_runtime::Runtime,
    shapes: apt_scene::Shapes,
    textures: BTreeMap<String, Handle<Image>>,
    slots: Vec<Slot>,
    failed: bool,
}
pub(crate) struct ScoringHudPlugin;
fn embed_render_shader(app: &mut App) {
    embedded_asset!(app, "hud_render.wgsl");
}
impl Plugin for ScoringHudPlugin {
    fn build(&self, app: &mut App) {
        embed_render_shader(app);
        app.add_plugins(Material2dPlugin::<HudMaterial>::default())
        // In the headset: read on a worker (it took the first frame ~4.5 s on the main thread) and shown on a panel
        // by a controller (`xr::hand_hud`).
        .add_systems(PostStartup, xr::start.run_if(|config: Res<Config>| config.xr.is_some()))
        .add_systems(
            Update,
            xr::finish
                .run_if(resource_exists::<xr::Loading>)
                .in_set(crate::app::SKATE3),
        )
        .add_systems(
            FixedUpdate,
            (advance, multiplier_sounds)
                .after(crate::app::SimulationSet::Physics)
                .in_set(crate::app::SKATE3),
        )
        .add_systems(
            Update,
            render
                .after(crate::app::FrameSet::Physics)
                .in_set(crate::app::SKATE3),
        );
    }
}
/// Where the player's prepared HUD is.
fn hud_root(config: &Config) -> PathBuf {
    std::env::var_os("SKATE_SCORING_HUD_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| config.asset_root.join("private/hud"))
}

/// The HUD read from its files (all but the GPU's part): its movie started, its shapes and its textures' pixels.
struct Loaded {
    runtime: hud_runtime::Runtime,
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
    Ok(Loaded { runtime, shapes, textures })
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
            runtime: self.runtime,
            shapes: self.shapes,
            textures,
            slots: Vec::new(),
            failed: false,
        }
    }
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
        let hud = loaded.into_hud(&mut app.world_mut().resource_mut::<Assets<Image>>());
        let target = app.world_mut().resource_mut::<Assets<Image>>().add(Image::new_target_texture(
            width as u32,
            height as u32,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
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
}
fn advance(
    hud: Option<ResMut<Hud>>,
    skater: Res<SkaterRuntime>,
) {
    let Some(mut hud) = hud else {
        return;
    };
    if hud.failed {
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
    ) {
        let Some(result) = block_on(future::poll_once(&mut loading.0)) else {
            return;
        };
        commands.remove_resource::<Loading>();
        let loaded = match result {
            Ok(loaded) => loaded,
            Err(error) => {
                error!("Original scoring HUD could not load from {error}");
                return;
            }
        };
        let hud = loaded.into_hud(&mut images);
        let [x, y, width, height] = AREA;
        let target = images.add(Image::new_target_texture(
            (width * PIXELS_PER_UNIT) as u32,
            (height * PIXELS_PER_UNIT) as u32,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
        let camera = commands
            .spawn((
                engine_api::EngineHudCamera,
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
            engine_api::HandHud {
                // (the trick display, in the area's left part, just over the controller: the area's bottom left a
                // little left of the grip and up)
                offset: OFFSET,
                anchor: Vec2::new(-0.5, -0.5),
                ..engine_api::HandHud::above(WIDTH_M, height / width, Some(camera))
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
