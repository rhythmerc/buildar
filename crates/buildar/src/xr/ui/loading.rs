//! The loading card: while the running engine gets ready, a small card ahead of the player says so (its name, what
//! it is doing, a bar sweeping); once its player can be put in the room, it says how, and the right controller aims
//! to place it (`xr::spawn::PlaceSkater`). Loading is done when the player can place, not when the engine is made:
//! a game can load for seconds after (`engine_driver::Ready`). Gone once the player is placed, and while a menu is
//! open.
//!
//! In the menu's look (`super::spawn_ui`): a dark rounded card, the title heavy, the line under it in the menu's
//! teal. It follows the head lazily, a little below the eyes.
use super::{DIM, TEAL, UI_BODY_FONT, UI_FONT};
use crate::engine_driver::{Readiness, Ready};
use crate::xr::panel::{XrPanel, XrPanelCamera};
use crate::xr::rig::Rig;
use bevy::camera::RenderTarget;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy_mod_xr::camera::XrCamera;

const WIDTH_PX: u32 = 800;
const HEIGHT_PX: u32 = 260;
const WIDTH_M: f32 = 0.32;
/// Where it sits in head space, metres: right, down, ahead.
const OFFSET: Vec3 = Vec3::new(0.0, -0.1, -0.8);
/// How quickly it catches up with the head, per second.
const FOLLOW: f32 = 4.0;
/// The bar's sweep: the moving part's share of the track, and its period, seconds.
const SWEEP_PART: f32 = 0.3;
const SWEEP_S: f32 = 1.6;

#[derive(Component)]
struct CardQuad;
#[derive(Component)]
struct CardCamera;
#[derive(Component)]
struct CardTitle;
#[derive(Component)]
struct CardLine;
#[derive(Component)]
struct CardTrack;
#[derive(Component)]
struct CardSweep;

pub(crate) struct LoadingCardPlugin;

impl Plugin for LoadingCardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Readiness>()
            .add_systems(Startup, setup)
            .add_systems(Update, (arm_placing, draw).chain().after(super::UiSet))
            .add_systems(PostUpdate, place.before(bevy::transform::TransformSystems::Propagate));
    }
}

/// The running engine's readiness, if it is the running one's (another, not hosted by the driver, says nothing).
fn current(readiness: &Readiness, engines: Option<&crate::engines::Engines>) -> Option<(crate::engines::Engine, Ready)> {
    readiness.0.filter(|(engine, _)| engines.and_then(|e| e.current()) == Some(*engine))
}

/// What the card says, if it shows: its title and its line, and whether the bar sweeps.
fn says(engine: crate::engines::Engine, ready: Ready, placing: bool) -> Option<(String, String, bool)> {
    let noun = engine.player_noun();
    let name = engine.name();
    match ready {
        Ready::Starting => Some((name.into(), "Loading the game".into(), true)),
        Ready::Preparing => Some((name.into(), "Getting it ready".into(), true)),
        Ready::ToPlace if placing => {
            Some((format!("Place your {noun}"), "Point with the right controller, then pull its trigger".into(), false))
        }
        Ready::ToPlace => Some((
            format!("Place your {noun}"),
            format!("From the menu: Place the {noun}. Or hold the left grip and trigger, and point"),
            false,
        )),
        Ready::Placed => None,
    }
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>, assets: Res<AssetServer>) {
    let mut image = Image::new_target_texture(WIDTH_PX, HEIGHT_PX, TextureFormat::Rgba8UnormSrgb, None);
    image.sampler = ImageSampler::linear();
    let image = images.add(image);
    let camera = commands
        .spawn((
            XrPanelCamera,
            CardCamera,
            Camera2d,
            Camera { order: -12, clear_color: ClearColorConfig::Custom(Color::NONE), is_active: false, ..default() },
            RenderTarget::Image(image.clone().into()),
        ))
        .id();
    let height_m = WIDTH_M * HEIGHT_PX as f32 / WIDTH_PX as f32;
    commands.spawn((
        CardQuad,
        Mesh3d(meshes.add(Rectangle::new(WIDTH_M, height_m))),
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
    spawn_card(&mut commands, camera, &assets);
}

/// The card's UI, drawn by `camera`.
fn spawn_card(commands: &mut Commands, camera: Entity, assets: &AssetServer) {
    let heavy = TextFont { font: assets.load(UI_FONT), font_size: 52.0, weight: FontWeight(800), ..default() };
    let body = TextFont { font: assets.load(UI_BODY_FONT), font_size: 28.0, ..default() };
    commands
        .spawn((
            Node { width: percent(100), height: percent(100), padding: UiRect::all(px(10)), ..default() },
            UiTargetCamera(camera),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: percent(100),
                    height: percent(100),
                    padding: UiRect::axes(px(34), px(24)),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    row_gap: px(10),
                    border_radius: BorderRadius::all(px(24)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.03, 0.05, 0.07, 0.9)),
            ))
            .with_children(|card| {
                card.spawn((CardTitle, Text::new(""), heavy, TextColor(Color::WHITE)));
                card.spawn((CardLine, Text::new(""), body, TextColor(TEAL)));
                card.spawn((
                    CardTrack,
                    Node {
                        width: percent(100),
                        height: px(8),
                        margin: UiRect::top(px(8)),
                        overflow: Overflow::clip(),
                        border_radius: BorderRadius::all(px(4)),
                        ..default()
                    },
                    BackgroundColor(DIM.with_alpha(0.18)),
                ))
                .with_children(|track| {
                    track.spawn((
                        CardSweep,
                        Node {
                            position_type: PositionType::Absolute,
                            width: percent(SWEEP_PART * 100.0),
                            height: percent(100),
                            border_radius: BorderRadius::all(px(4)),
                            ..default()
                        },
                        BackgroundColor(TEAL),
                    ));
                });
            });
        });
}

/// Once the engine can take its player (and no menu is up): the right controller aims to place it, once.
fn arm_placing(
    readiness: Res<Readiness>,
    engines: Option<Res<crate::engines::Engines>>,
    panel: Res<XrPanel>,
    build: Res<crate::xr::build::BuildMode>,
    mut placing: ResMut<crate::xr::spawn::PlaceSkater>,
    mut armed: Local<Option<crate::engines::Engine>>,
) {
    let Some((engine, Ready::ToPlace)) = current(&readiness, engines.as_deref()) else { return };
    if *armed != Some(engine) && !panel.visible && !build.active {
        *armed = Some(engine);
        placing.0 = true;
    }
}

#[allow(clippy::type_complexity)]
fn draw(
    readiness: Res<Readiness>,
    engines: Option<Res<crate::engines::Engines>>,
    placing: Res<crate::xr::spawn::PlaceSkater>,
    time: Res<Time<Real>>,
    mut drawn: Local<Option<(String, String, bool)>>,
    mut texts: ParamSet<(Query<&mut Text, With<CardTitle>>, Query<&mut Text, With<CardLine>>)>,
    mut track: Query<&mut Node, (With<CardTrack>, Without<CardSweep>)>,
    mut sweep: Query<&mut Node, (With<CardSweep>, Without<CardTrack>)>,
) {
    let said = current(&readiness, engines.as_deref()).and_then(|(engine, ready)| says(engine, ready, placing.0));
    let Some((title, line, sweeping)) = said else {
        *drawn = None;
        return;
    };
    if sweeping {
        // (the moving part enters at the left and leaves at the right)
        let t = (time.elapsed_secs() / SWEEP_S).fract();
        let left = -SWEEP_PART + t * (1.0 + SWEEP_PART);
        for mut node in &mut sweep {
            node.left = percent(left * 100.0);
        }
    }
    if drawn.as_ref() == Some(&(title.clone(), line.clone(), sweeping)) {
        return;
    }
    for mut t in &mut texts.p0() {
        **t = title.clone();
    }
    for mut t in &mut texts.p1() {
        **t = line.clone();
    }
    for mut node in &mut track {
        node.display = if sweeping { Display::Flex } else { Display::None };
    }
    *drawn = Some((title, line, sweeping));
}

#[allow(clippy::type_complexity)]
fn place(
    rig: Res<Rig>,
    panel: Res<XrPanel>,
    readiness: Res<Readiness>,
    engines: Option<Res<crate::engines::Engines>>,
    placing: Res<crate::xr::spawn::PlaceSkater>,
    time: Res<Time<Real>>,
    eyes: Query<&Transform, (With<XrCamera>, Without<CardQuad>)>,
    mut quad: Query<(&mut Transform, &mut Visibility), With<CardQuad>>,
    mut camera: Query<&mut Camera, With<CardCamera>>,
    mut pose: Local<Option<Transform>>,
) {
    let (Ok((mut tf, mut vis)), Ok(mut cam)) = (quad.single_mut(), camera.single_mut()) else { return };
    let (mut head, mut n, mut rotation) = (Vec3::ZERO, 0.0, None);
    for eye in &eyes {
        head += eye.translation;
        rotation.get_or_insert(eye.rotation);
        n += 1.0;
    }
    let shows = current(&readiness, engines.as_deref()).and_then(|(e, r)| says(e, r, placing.0)).is_some();
    let show = shows && !panel.visible && rotation.is_some();
    if cam.is_active != show {
        cam.is_active = show;
    }
    vis.set_if_neq(if show { Visibility::Visible } else { Visibility::Hidden });
    let (Some(rotation), Some(root), true) = (rotation, rig.root_transform(), show) else {
        *pose = None;
        return;
    };
    let head = head / n;
    // (level with the horizon: turned with the head's yaw only, so it doesn't swing as the head tilts)
    let forward = (rotation * Vec3::NEG_Z).with_y(0.0).normalize_or(Vec3::NEG_Z);
    let level = Transform::default().looking_to(forward, Vec3::Y).rotation;
    let at = head + level * OFFSET;
    let target = Transform::from_translation(at).looking_to(at - head, Vec3::Y);
    let k = 1.0 - (-FOLLOW * time.delta_secs()).exp();
    let current = pose.get_or_insert(target);
    current.translation = current.translation.lerp(target.translation, k);
    current.rotation = current.rotation.slerp(target.rotation, k);
    *tf = root.mul_transform(*current);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::Engine;

    /// Loading until the player can place, then how to place (the aiming, or how to start it); nothing once placed.
    #[test]
    fn the_card_says_loading_until_the_player_can_place() {
        assert_eq!(says(Engine::Mkwii, Ready::Starting, false).unwrap().2, true);
        assert_eq!(says(Engine::Mkwii, Ready::Preparing, false).unwrap().2, true);
        let (title, line, sweeping) = says(Engine::Mkwii, Ready::ToPlace, true).unwrap();
        assert_eq!((title.as_str(), sweeping), ("Place your kart", false));
        assert!(line.contains("trigger"), "{line}");
        assert!(says(Engine::Thug2, Ready::ToPlace, false).unwrap().1.contains("Place the skater"));
        assert!(says(Engine::Mkwii, Ready::Placed, false).is_none());
    }

    /// The card as the headset draws it, saved as a picture to look at:
    /// `SK8_CARD_SHOT=/path/card.png cargo test ... a_picture_of_the_loading_card -- --ignored`
    /// (`SK8_CARD_PLACE=1`: the prompt to place, else loading).
    #[test]
    #[ignore = "needs a GPU; writes a picture"]
    fn a_picture_of_the_loading_card() {
        use bevy::render::view::screenshot::{Screenshot, save_to_disk};
        let Some(path) = std::env::var_os("SK8_CARD_SHOT") else { return };
        let (title, line, sweeping) =
            says(Engine::Mkwii, if std::env::var_os("SK8_CARD_PLACE").is_some() { Ready::ToPlace } else { Ready::Preparing }, true).unwrap();
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .build()
                .disable::<bevy::winit::WinitPlugin>()
                .set(WindowPlugin { primary_window: None, exit_condition: bevy::window::ExitCondition::DontExit, ..default() }),
        );
        crate::fonts::embed(&mut app);
        let image = Image::new_target_texture(WIDTH_PX, HEIGHT_PX, TextureFormat::Rgba8UnormSrgb, None);
        let image = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        // (over a mid grey, as over a room)
        let camera = app
            .world_mut()
            .spawn((Camera2d, Camera { clear_color: ClearColorConfig::Custom(Color::srgb(0.45, 0.45, 0.42)), ..default() }, RenderTarget::Image(image.clone().into())))
            .id();
        app.add_systems(Startup, move |mut commands: Commands, assets: Res<AssetServer>| spawn_card(&mut commands, camera, &assets));
        app.add_systems(
            Update,
            move |mut texts: ParamSet<(Query<&mut Text, With<CardTitle>>, Query<&mut Text, With<CardLine>>)>,
                  mut track: Query<&mut Node, (With<CardTrack>, Without<CardSweep>)>,
                  mut sweep: Query<&mut Node, (With<CardSweep>, Without<CardTrack>)>| {
                for mut t in &mut texts.p0() {
                    **t = title.clone();
                }
                for mut t in &mut texts.p1() {
                    **t = line.clone();
                }
                for mut n in &mut track {
                    n.display = if sweeping { Display::Flex } else { Display::None };
                }
                for mut n in &mut sweep {
                    n.left = percent(30.0);
                }
            },
        );
        app.finish();
        app.cleanup();
        for _ in 0..30 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        app.world_mut().spawn(Screenshot::image(image)).observe(save_to_disk(std::path::PathBuf::from(path)));
        for _ in 0..30 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
