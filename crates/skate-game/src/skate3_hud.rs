//! Skate 3's HUD in the headset: the retail trick display (`skate3_engine::hud`: read from the player's prepared HUD,
//! advanced with the scoring) drawn by the app's HUD renderer (`engine_hud`) on a panel by a controller. Read on a
//! worker (it took the first frame ~4.5 s on the main thread). Without the HUD files there is no panel, and the
//! corner readout (`xr::ui::hud`) shows the tricks instead. The flat desktop run has none (legacy pass 4).
//!
//! And the score multiplier's sounds, which retail plays from this HUD module (whether or not the display loaded).
use crate::app::{FrameSet, SKATE3, SimulationSet};
use crate::config::Config;
use crate::engines::Engine;
use crate::physics::SkaterRuntime;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future};
use engine_core::hud::HudFrame;
use skate3_engine::hud::Hud;

pub(crate) struct Skate3HudPlugin;

impl Plugin for Skate3HudPlugin {
    fn build(&self, app: &mut App) {
        crate::engine_hud::install(app);
        app.add_systems(PostStartup, start.run_if(|config: Res<Config>| config.xr.is_some()))
            .add_systems(Update, finish.run_if(resource_exists::<Loading>).in_set(SKATE3))
            .add_systems(FixedUpdate, (advance, multiplier_sounds).after(SimulationSet::Physics).in_set(SKATE3))
            .add_systems(Update, draw.after(FrameSet::Physics).in_set(SKATE3));
    }
}

/// The HUD being read.
#[derive(Resource)]
struct Loading(Task<Result<Hud, String>>);

/// The HUD read, and its last frame.
#[derive(Resource)]
struct Skate3Hud {
    hud: Hud,
    frame: HudFrame,
}

fn start(mut commands: Commands, config: Res<Config>, skater: Res<SkaterRuntime>) {
    let root = skate3_engine::hud::root(&config.asset_root);
    let input = skater.scoring.hud_input();
    commands.insert_resource(Loading(
        AsyncComputeTaskPool::get().spawn(async move { Hud::load(&root, input).map_err(|e| format!("{}: {e}", root.display())) }),
    ));
}

fn finish(mut commands: Commands, mut loading: ResMut<Loading>) {
    let Some(result) = block_on(future::poll_once(&mut loading.0)) else {
        return;
    };
    commands.remove_resource::<Loading>();
    match result {
        Ok(hud) => {
            commands.insert_resource(Skate3Hud { hud, frame: HudFrame::default() });
            info!("Original scoring HUD loaded (headset)");
        }
        Err(error) => error!("Original scoring HUD could not load from {error}"),
    }
}

fn advance(hud: Option<ResMut<Skate3Hud>>, skater: Res<SkaterRuntime>) {
    let Some(mut hud) = hud else { return };
    let scoring = &skater.scoring;
    if let Err(error) = hud.hud.advance(scoring.hud_input(), scoring.new_trick, scoring.modified_trick, scoring.close_tricks) {
        error!("Original scoring HUD stopped: {error}");
    }
}

fn draw(hud: Option<ResMut<Skate3Hud>>, mut renderer: crate::engine_hud::HudRenderer) {
    let Some(mut hud) = hud else { return };
    let Skate3Hud { hud, frame } = &mut *hud;
    match hud.frame(frame) {
        Ok(true) => {
            renderer.show(Engine::Skate3, frame, |id| hud.texture(id));
        }
        Ok(false) => {}
        Err(error) => {
            error!("Original HUD geometry: {error}");
            // (it stopped: nothing shown from now on - the frame has its part and no draws)
            renderer.show(Engine::Skate3, frame, |_| None);
        }
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
fn multiplier_sounds(skater: Res<SkaterRuntime>, mut sounds: MessageWriter<crate::skate_audio::FrontEndSoundRequest>) {
    let Some(multiplier) = skater.scoring.multiplier_changed else {
        return;
    };
    if let Some(id) = crate::skate_audio::player::multiplier_sound_id(multiplier) {
        sounds.write(crate::skate_audio::FrontEndSoundRequest(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skate3_sim::{apt_vm, hud_runtime};

    /// The HUD through a line - a kickflip, then a modification of it, a second trick and a multiplier - as the
    /// headset draws it (its part: the trick display's corner), saved as pictures to look at:
    /// `SKATE_SCORING_HUD_ROOT=<assets>/private/hud SK8_HUD_SHOT=/path/hud.png cargo test ... a_picture_of_the_hud
    /// -- --ignored` writes `hud-f010.png` ... (and `.rgba`, raw) at ten points of it (`SK8_HUD_SHOT_FULL=1`: the
    /// whole stage).
    #[test]
    #[ignore = "needs a GPU and the player's HUD; writes pictures"]
    fn a_picture_of_the_hud() {
        let (Some(path), Some(root)) = (std::env::var_os("SK8_HUD_SHOT"), std::env::var_os("SKATE_SCORING_HUD_ROOT")) else {
            return;
        };
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
                apt_vm::Value::Text(name.into()),
                apt_vm::Value::Bool(false),
                apt_vm::Value::Bool(false),
                apt_vm::Value::Bool(false),
                apt_vm::Value::Bool(new),
            ],
            context_tricks: Vec::new(),
        };
        let mut hud = Hud::load(std::path::Path::new(&root), input("", 0, 1.0, false)).expect("the HUD");
        let full = std::env::var_os("SK8_HUD_SHOT_FULL").is_some();
        let mut frames = Vec::new();
        for frame in 0..150 {
            let (input, new, modified) = match frame {
                0..20 => (input("", 0, 1.0, false), false, false),
                20 => (input("Kickflip", 500, 1.0, true), true, false),
                21..60 => (input("Kickflip", 500, 1.0, false), false, false),
                60 => (input("Kickflip to Nosegrind", 1450, 2.0, false), false, true),
                61..100 => (input("Kickflip to Nosegrind", 1450, 2.0, false), false, false),
                100 => (input("Hardflip", 3250, 3.0, true), true, false),
                _ => (input("Hardflip", 3250, 3.0, false), false, false),
            };
            hud.advance(input, new, modified, false).expect("a HUD frame");
            let mut drawn = HudFrame::default();
            assert!(hud.frame(&mut drawn).expect("drawn"), "advanced: drawn again");
            if [10, 25, 40, 59, 65, 80, 99, 104, 120, 149].contains(&frame) {
                if full {
                    drawn.parts[0].size = drawn.canvas;
                    drawn.parts[0].view = drawn.canvas / 2.0;
                }
                frames.push((format!("f{frame:03}"), drawn));
            }
        }
        let mut textures = std::collections::HashMap::new();
        for id in 0.. {
            let Some(texture) = hud.texture(id) else { break };
            textures.insert(id, texture);
        }
        let path = std::path::PathBuf::from(path);
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("hud").to_string();
        let pictures: Vec<_> = crate::engine_hud::tests::pictures(Engine::Skate3, &frames, textures)
            .into_iter()
            .map(|(name, width, height, pixels)| (format!("{stem}-{}", name.trim_end_matches("-trick display")), width, height, pixels))
            .collect();
        assert_eq!(pictures.len(), frames.len());
        crate::engine_hud::tests::save(path.parent().unwrap_or(std::path::Path::new(".")), &pictures);
    }
}
