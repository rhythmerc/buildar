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
pub(crate) const WIDTH_M: f32 = 0.62 * DISTANCE_M / 0.7;
pub(crate) const HEIGHT_M: f32 = WIDTH_M * HEIGHT_PX as f32 / WIDTH_PX as f32;
/// How far in front of the eyes it opens, and how far below them, metres. A metre: Meta's guidance for UI worked with
/// a ray is out of the "middle distance" (0.5 to 0.8 m), at a metre or more. (It was 0.7 m and 0.62 m wide: it's as
/// big to the eye as it was, as are its drop and its keyboard's place under it.)
const DISTANCE_M: f32 = 1.0;
const DROP_M: f32 = 0.12 * DISTANCE_M / 0.7;

/// Marks the panel's UI camera, which `headset_only_rendering` leaves on.
#[derive(Component)]
pub(crate) struct XrPanelCamera;

#[derive(Component)]
struct PanelQuad;

#[derive(Resource)]
pub(crate) struct XrPanel {
    pub camera: Entity,
    /// Show the quad (and render its camera).
    pub visible: bool,
    /// Place it in front of the head on the next frame.
    pub recenter: bool,
    /// Tracking-space pose of the quad.
    pose: Transform,
}

impl XrPanel {
    /// A panel drawn by `camera`, for a test that renders the menus without a headset.
    #[cfg(test)]
    pub(crate) fn drawn_by(camera: Entity) -> Self {
        Self { camera, visible: true, recenter: false, pose: Transform::default() }
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
        app.add_systems(PreStartup, setup)
            .add_systems(PostUpdate, place.before(bevy::transform::TransformSystems::Propagate))
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
        ))
        .id();
    commands.spawn((
        PanelQuad,
        Mesh3d(meshes.add(Rectangle::new(WIDTH_M, HEIGHT_M))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(image),
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

fn place(
    rig: Res<Rig>,
    mut panel: ResMut<XrPanel>,
    eyes: Query<&Transform, (With<XrCamera>, Without<PanelQuad>)>,
    mut quad: Query<(&mut Transform, &mut Visibility), With<PanelQuad>>,
    mut cameras: Query<&mut Camera, With<XrPanelCamera>>,
) {
    if let Ok(mut camera) = cameras.get_mut(panel.camera) {
        if camera.is_active != panel.visible {
            camera.is_active = panel.visible;
        }
    }
    let Ok((mut tf, mut vis)) = quad.single_mut() else { return };
    let want = if panel.visible { Visibility::Visible } else { Visibility::Hidden };
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
