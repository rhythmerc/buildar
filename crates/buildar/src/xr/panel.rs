//! A world-space UI panel for XR: a UI camera renders into an image shown on
//! a quad that opens in front of the player. There is no flat HUD in XR, so
//! menus target [`XrPanel::camera`] (with `UiTargetCamera`) instead.
//!
//! The quad is kept in tracking space (real metres), so it stays put in the
//! room while the world is rescaled.
use super::rig::Rig;
use bevy::camera::RenderTarget;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy_mod_xr::camera::XrCamera;

/// Panel image size, pixels, and its real size, metres.
pub(crate) const WIDTH_PX: u32 = 1000;
pub(crate) const HEIGHT_PX: u32 = 900;
pub(crate) const WIDTH_M: f32 = 0.62;
pub(crate) const HEIGHT_M: f32 = WIDTH_M * HEIGHT_PX as f32 / WIDTH_PX as f32;
/// How far in front of the eyes it opens, and how far below them, metres. A metre: Meta's guidance for UI worked with
/// a ray is out of the "middle distance" (0.5 to 0.8 m), at a metre or more. (It was 0.7 m out; it's kept its
/// size, 0.62 m wide - as big to the eye as it was took too much of the view - and its drop is as it was to the eye.)
const DISTANCE_M: f32 = 1.0;
const DROP_M: f32 = 0.12 * DISTANCE_M / 0.7;

/// Marks the panel's UI camera, which `headset_only_rendering` leaves on.
#[derive(Component)]
pub(crate) struct XrPanelCamera;

/// The panel's quad in the scene (its picture, or where it's a compositor layer, the hole it shows through:
/// `panel_layer`).
#[derive(Component)]
pub(crate) struct PanelQuad;

/// The panel's picture is drawn again this many frames more (the UI changed, or it was just shown): a menu that
/// isn't changing isn't drawn each frame - its UI camera and the copy into the compositor layer cost every frame
/// they ran. (A few frames, as what changed reaches the picture over more than one: layout, then text, then images.)
#[derive(Resource, Default)]
pub(crate) struct PanelRedraw {
    pub frames: u32,
    /// The picture was drawn this frame (the camera ran): the layer copies it.
    pub drawn: bool,
    since: f32,
}

/// Frames drawn after a change, and the most seconds between draws however still the menu is (in case a change
/// isn't seen).
const REDRAW_FRAMES: u32 = 4;
const REDRAW_EVERY_S: f32 = 1.0;

/// Where the panel is put each frame (`panel_layer` reads it after).
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) struct PanelSystems;

#[derive(Resource)]
pub(crate) struct XrPanel {
    pub camera: Entity,
    /// Show the quad (and render its camera).
    pub visible: bool,
    /// Place it in front of the head on the next frame.
    pub recenter: bool,
    /// Tracking-space pose of the quad.
    pose: Transform,
    /// The picture the UI camera draws.
    pub image: Handle<Image>,
    /// The panel is a compositor layer (`panel_layer`): its quad in the scene is hidden for the hole beside it.
    pub layered: bool,
}

impl XrPanel {
    /// A panel drawn by `camera`, for a test that renders the menus without a headset.
    #[cfg(test)]
    pub(crate) fn drawn_by(camera: Entity) -> Self {
        Self { camera, visible: true, recenter: false, pose: Transform::default(), image: Handle::default(), layered: false }
    }

    /// Tracking-space pose of the quad: its front (+Z) faces the player,
    /// +X is their right.
    pub(crate) fn pose(&self) -> Transform {
        self.pose
    }
}

pub(crate) struct PanelPlugin;

impl Plugin for PanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PanelRedraw>()
            .add_systems(PreStartup, setup)
            .add_systems(
                PostUpdate,
                (watch_ui.after(bevy::ui::UiSystems::Layout), place)
                    .chain()
                    .in_set(PanelSystems)
                    .before(bevy::transform::TransformSystems::Propagate),
            )
            .add_systems(Last, report_on_open);
    }
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut image = Image::new_target_texture(WIDTH_PX, HEIGHT_PX, TextureFormat::Rgba8UnormSrgb, None);
    image.sampler = ImageSampler::linear();
    // (copied into the compositor layer's swapchain: `panel_layer`)
    image.texture_descriptor.usage |= bevy::render::render_resource::TextureUsages::COPY_SRC;
    let image = images.add(image);
    let camera = commands
        .spawn((
            XrPanelCamera,
            Camera2d,
            Camera {
                order: -10,
                clear_color: ClearColorConfig::Custom(Color::NONE),
                ..default()
            },
            RenderTarget::Image(image.clone().into()),
            // (UI draws its own smooth edges: the default 4x was a multisampled colour and depth pass for nothing)
            Msaa::Off,
        ))
        .id();
    commands.spawn((
        PanelQuad,
        Mesh3d(meshes.add(Rectangle::new(WIDTH_M, HEIGHT_M))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(image.clone()),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            ..default()
        })),
        Transform::default(),
        Visibility::Hidden,
    ));
    commands.insert_resource(XrPanel {
        camera,
        visible: false,
        recenter: true,
        pose: Transform::from_xyz(0.0, 1.2, -DISTANCE_M),
        image,
        layered: false,
    });
}

/// Where the head is (between the eyes) and how it's turned, once the headset tracks it: eyes still at the tracking
/// origin, unturned, aren't tracked yet.
fn tracked_head<'a>(eyes: impl Iterator<Item = &'a Transform>) -> Option<(Vec3, Quat)> {
    let (mut sum, mut n, mut rotation) = (Vec3::ZERO, 0.0, None);
    for eye in eyes {
        if eye.translation == Vec3::ZERO && eye.rotation == Quat::IDENTITY {
            continue;
        }
        sum += eye.translation;
        rotation.get_or_insert(eye.rotation);
        n += 1.0;
    }
    Some((sum / n, rotation?))
}

/// Whether the menu's UI changed this frame (after layout): any node added, removed or changed, its layout, text,
/// colours, image or visibility, or an image loaded or changed.
#[allow(clippy::type_complexity)]
fn watch_ui(
    changed: Query<
        (),
        (
            With<Node>,
            Or<(
                Changed<Node>,
                Changed<ComputedNode>,
                Changed<UiGlobalTransform>,
                Changed<Text>,
                Changed<TextColor>,
                Changed<TextFont>,
                Changed<BackgroundColor>,
                Changed<BorderColor>,
                Changed<ImageNode>,
                Changed<InheritedVisibility>,
            )>,
        ),
    >,
    mut removed: RemovedComponents<Node>,
    mut images: MessageReader<AssetEvent<Image>>,
    panel: Res<XrPanel>,
    time: Res<Time<Real>>,
    mut redraw: ResMut<PanelRedraw>,
    mut was_visible: Local<bool>,
) {
    let images_changed = images.read().any(|e| !matches!(e, AssetEvent::Unused { .. }));
    let shown = panel.visible && !std::mem::replace(&mut *was_visible, panel.visible);
    redraw.since += time.delta_secs();
    if shown || !changed.is_empty() || removed.read().count() > 0 || images_changed || redraw.since >= REDRAW_EVERY_S {
        redraw.frames = REDRAW_FRAMES;
    }
    redraw.drawn = panel.visible && redraw.frames > 0;
    if redraw.drawn {
        redraw.frames -= 1;
        redraw.since = 0.0;
    }
}

fn place(
    rig: Res<Rig>,
    redraw: Res<PanelRedraw>,
    mut panel: ResMut<XrPanel>,
    eyes: Query<&Transform, (With<XrCamera>, Without<PanelQuad>)>,
    mut quad: Query<(&mut Transform, &mut Visibility), With<PanelQuad>>,
    mut cameras: Query<&mut Camera, With<XrPanelCamera>>,
) {
    if let Ok(mut camera) = cameras.get_mut(panel.camera) {
        // (drawn only while the menu is changing: `PanelRedraw`)
        let active = panel.visible && redraw.drawn;
        if camera.is_active != active {
            camera.is_active = active;
        }
    }
    let Ok((mut tf, mut vis)) = quad.single_mut() else { return };
    let want = if panel.visible && !panel.layered { Visibility::Visible } else { Visibility::Hidden };
    if *vis != want {
        *vis = want;
    }
    if panel.recenter {
        // (not before the headset has the head: at launch the eyes sit at the tracking origin a few frames, and a
        // panel put in front of that is on the floor, out of sight)
        if let Some((head, rotation)) = tracked_head(eyes.iter()) {
            panel.recenter = false;
            let forward = (rotation * Vec3::NEG_Z).with_y(0.0).normalize_or(Vec3::NEG_Z);
            let at = head + forward * DISTANCE_M - Vec3::Y * DROP_M;
            // Face the player: -Z looks away, so the quad's front (+Z) faces the head.
            panel.pose = Transform::from_translation(at).looking_to(forward, Vec3::Y);
        }
    }
    let Some(root) = rig.root_transform() else { return };
    let target = root.mul_transform(panel.pose);
    if *tf != target {
        *tf = target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The menu opened at launch waits for the head: eyes at the tracking origin aren't it.
    #[test]
    fn the_head_is_where_tracked_eyes_are() {
        assert_eq!(tracked_head([Transform::default(), Transform::default()].iter()), None);
        let left = Transform::from_xyz(-0.03, 1.6, 0.2).with_rotation(Quat::from_rotation_y(0.3));
        let right = Transform::from_xyz(0.03, 1.6, 0.2).with_rotation(Quat::from_rotation_y(0.3));
        let (head, turn) = tracked_head([left, right].iter()).unwrap();
        assert!(head.distance(Vec3::new(0.0, 1.6, 0.2)) < 1e-6 && turn == left.rotation);
    }
}

/// Each time the menu opens, its state to the log (`PANEL ...`): what draws it - its camera, its quad, the menu's root
/// UI node - and whether any of them is an engine's (hidden with that engine), with the engine running. To find why a
/// menu shows nothing on the headset.
#[allow(clippy::type_complexity)]
fn report_on_open(
    panel: Res<XrPanel>,
    engines: Option<Res<crate::engines::Engines>>,
    cameras: Query<(&Camera, Option<&crate::engines::EngineOwned>)>,
    quad: Query<(&Visibility, &ViewVisibility, Option<&crate::engines::EngineOwned>), With<PanelQuad>>,
    roots: Query<(&Node, &Visibility, Option<&crate::engines::EngineOwned>), With<crate::xr::ui::UiRoot>>,
    mut was: Local<bool>,
) {
    if panel.visible == std::mem::replace(&mut *was, panel.visible) || !panel.visible {
        return;
    }
    let camera = cameras.get(panel.camera).map(|(c, o)| format!("active {} owner {:?}", c.is_active, o.map(|o| o.0)));
    let quad = quad.single().map(|(v, seen, o)| format!("{v:?} seen {} owner {:?}", seen.get(), o.map(|o| o.0)));
    let roots: Vec<String> = roots.iter().map(|(n, v, o)| format!("{:?} {v:?} owner {:?}", n.display, o.map(|o| o.0))).collect();
    info!(
        "PANEL opened with {:?} running: camera {camera:?}; quad {quad:?}; menu roots {roots:?}",
        engines.and_then(|e| e.current())
    );
}
