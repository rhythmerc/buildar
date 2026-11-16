# Skate 3 as a standalone, Bevy-free sim (thug2-headless shape)

Status: scoping study, 2026-11-16. Research only: no code was changed. Builds on
`docs/engine-packaging-proposal.md` (the audit: crate layout, coupling points, steps 0-9). That document's
step 9 is now the target. Its steps 0-3 stay as written. Its step 6 shrinks to "adapter + renderer".

Snapshot: `main` at `11a93ca` + working tree. Line counts are `wc -l` of `src/**/*.rs` and include
in-file tests. Nothing was compiled for this study, because cargo is only in the distrobox.

---

## 1. Where the sim/render line falls

### 1.1 The tick already exists as a plain function

The per-tick pipeline is already a chain of plain Rust calls on plain structs. Bevy is only the container
around it:

```
FixedUpdate:
  SimulationSet::Input    input::publish_actions        ControllerInput -> PublishedTickInput (TickInput)
  (session_marker::update, between Input and Controls)
  SimulationSet::Controls physics::controls::sample      -> PlayerControls::update_for_physics(...)
  (multiplayer apply, between Controls and Physics)
  SimulationSet::Physics  physics::advance               -> physics::frame::advance(&mut GamePhysics,
                                                             &mut SkaterRuntime, &mut PlayerControls,
                                                             &StockGraphs, &mut dyn ActionMap, bool,
                                                             &mut CameraRuntime)
  after Physics: audio_observation::publish, presentation::capture, scoring_hud::{advance,
                 multiplier_sounds}, modding::fixed, multiplayer::send
Update: animation::present, physics::present, camera::present, scoring_hud::render, player_audio::forward
```

- The sim state is five structs: `GamePhysics` (`physics.rs:95`), `SkaterRuntime` (`physics/skater.rs:28`,
  60+ sub-runtimes), `PlayerControls` (`physics/controls.rs:12`), `StockGraphs` (`graph_runtime.rs:299`) and
  `CameraRuntime` (`camera/runtime.rs:25`). Each one is Bevy only through `#[derive(Resource)]`.
- Loading is already Bevy-free and runs on a worker thread: `lib.rs::load` (lines 197-300) calls
  `GamePhysics::load_with_difficulty`, `SkaterRuntime::load`, `PlayerControls::load`, `StockGraphs::load`
  and `GameAssets::load`. Its only Bevy use is `bevy::log::info_span`.
- Render outputs are already plain data. `SkaterRuntime.render_pose: Vec<NativeMatrix>` (native animation
  space), `animated_skeleton.roots.animation_to_world` (root), `pose_generation`,
  `board.part_transforms()`, `CameraRuntime.frame: Option<CameraFrame>`. `presentation.rs::capture` turns
  these into Bevy `Transform`s on the render side.
- The APT scoring HUD is already split. `hud_runtime.rs` + `apt_*.rs` (1,704 lines, **zero Bevy**) run the
  ActionScript VM and `apt_scene::draw` returns `Vec<Draw { texture, size, vertices, multiply, add }>`.
  `scoring_hud.rs` only turns those into `Mesh2d` + `HudMaterial`. That is the same split as
  `thug2_hud`/`thug2_hud_texture`.
- The sim modules have few upward dependencies on app modules (grep of `crate::` from the sim set):
  - `crate::animation::native_matrix`: a 6-line conversion, used in `physics/network.rs` and
    `physics/climbing/mod.rs`.
  - `crate::modding::vehicles::network::CollisionShape`: network proxies, in `physics/network.rs:350`.
  - `crate::crash_context::requested`: a diagnostic, in `physics/player_state/transition.rs:77`.
  - `crate::config::Config`: plugin builds only.
  - `skate_mods::TrainerTuning`: `physics.rs:120`, `frame.rs:95` and `ground_runtime/settings.rs:219`.

### 1.2 Classification

"Bevy touch" says what the Bevy dependency actually is. Counts include in-file tests.

| Module(s) (`crates/skate-game/src/`) | Lines | Bevy touch | Side |
|---|---:|---|---|
| `physics/` + `physics.rs` | 37,384 | 20 of 271 files: mostly `bevy::log` macros; `Resource` derives; glam via `use bevy::prelude::*` re-exported through `super::*` (`network.rs`, `climbing/` 1,463 lines incl. `Transform` in `clip.rs`); ECS glue in `physics.rs` (`PhysicsPlugin`, `advance`, `present`, ~250 lines), `controls.rs::sample`, `audio_observation.rs::publish` | **sim** (glue → adapter) |
| `graph_host/` | 14,941 | 1 file, `bevy::log::info!` (`action.rs:316`) | **sim** |
| `player_audio/` | 18,149 | none | **sim** |
| `player_audio.rs` | 1,888 | worker logic (`prepare`, `run`, `DirectMixer`, `EventProducer`, ~1,500) is plain; `start`/`forward`/`follow_skater`/`report` + `AudioPlayer`/`LivePcm` (~400) are output | split |
| `skate_audio.rs` | 1,270 | `PlayerAudioObservation`/`RetailAudioInputs`/`BodyContacts` (~330, `Message` derive) + XMA/bank decode fns (~150) are sim; `SkateAudioPlugin`, `LivePcm`, decode tasks are output | split |
| `scoring_runtime.rs` | 1,602 | none | **sim** |
| `hud_runtime.rs`, `apt_vm/scene/movie/text/display.rs` | 1,704 | none | **sim** (HUD as data) |
| `camera/` (stock camera graph, runtime, publication) | 1,857 | `Resource` derive, `bevy::log` | **sim** (feeds physics and offboard steering) |
| `grind_world/` + `grind_world.rs` | 1,151 | `grind_world.rs` 215: plugin + `Resource` | sim (plugin → adapter) |
| `input/` gesture + controllers | 1,233 | `controllers.rs` `Resource` derive | **sim** |
| `input/platform.rs` (XInput), `input.rs` (plugin, `ExternalPad`, `poll_controllers`) | 308 | ECS, `Window` focus | adapter/platform |
| `graph_runtime.rs`, `skater_animation*`, `animation_pose*` | 1,297 | `Resource` derive; `bevy::math` in `animation_pose/authored_clips.rs` | **sim** |
| `skate_world.rs` | 1,395 | `collision_world`/`validate_runtime` (~380 + 8 tests) plain; `spawn`/render batches (~580 + 8 tests) Bevy | split |
| `difficulty.rs`, `map_library.rs`, `setup.rs` | 460 | none | sim (data/bundle) |
| `src/tests/` | 7,088 | only `map_transition.rs` builds an `App` | sim (except that file) |
| `presentation.rs`, `replay/`, `animation.rs`, `world.rs`, `assets.rs`, `camera.rs` | 1,518 | Transform blending, GLB bone binding, Bevy camera | **render/adapter** |
| `retail_*.rs` + 13 `.wgsl` | 3,952 | renderer, materials, shadows, sky, exposure | **render** |
| `customiser*.rs`, `custom_models.rs` (+2 `.wgsl`) | 3,935 | GLB parts, materials, UI | **render/UI** |
| `map_render.rs`, `graphics_menu.rs`, `render_capacity.rs`, `performance.rs`, `fps_overlay.rs` | 2,016 | render | **render** |
| `scoring_hud.rs` + `hud_render.wgsl`, `hud_composite.wgsl` | 742 | `Material2d`, `UiMaterial`, XR panel | **render** |
| `session_marker/` | 877 | `effect.rs`/`hud.rs` render (447); `state`/`validation`/`noise` (220) logic | adapter (game mode) |
| `map_transition.rs`, `teleport_menu.rs` | 619 | orchestration/UI around sim loads | adapter |
| `modding/`, `multiplayer*`, `updater`, `crash_*`, `profiling`, `verification` | ~8,700 | desktop product | app / `skate3-desktop` |
| `xr/link_retail.rs` | 197 | the current XR adapter | adapter |

**Totals:** about **82k lines of sim** plus 7k of sim tests, all already Bevy-free or incidental. About
**25k lines** are render, adapter and desktop product, and they stay in the app.

### 1.3 De-Bevying the "incidental" group (S-M overall)

- **Logging:** `bevy::log::{info,warn,error,info_span}` becomes the `tracing` crate (`=0.1.44`, the same
  one `bevy_log` uses, so Bevy's `LogPlugin` still collects the events). 16 sim files use log macros,
  plus `lib.rs::load`. This is a mechanical change.
- **Math:** add `glam = "=0.30.10"` directly. That is exactly what `bevy_math` 0.18.1 resolves to
  (`Cargo.lock`), so `Vec3`/`Quat`/`Mat4` are the *same types* on both sides and need no conversion. About
  20 files (~3.8k lines) name glam types, mostly reached through `physics.rs`'s `use bevy::prelude::*`
  re-exported by `use super::*`. Remove that prelude import and add `use glam::*` where needed.
- **`Transform` in `physics/climbing/clip.rs`** (`sample`, `globals`, `blend`, `decode`): replace it with a
  local `{translation: Vec3, rotation: Quat}` (~40 lines).
- **`#[derive(Resource)]`:** drop it on `GamePhysics`, `SkaterRuntime`, `PlayerControls`, `StockGraphs`,
  `CameraRuntime`, `ControllerInput`, `OffboardView`, `AudioObservationCursor` and the grind provider.
  The adapter holds one `Skate3(Sim)` resource, or newtypes during the transition.
- **`#[derive(Message)]`** on `PlayerAudioObservation`/`FrontEndSoundRequest`: plain structs in the sim.
  The adapter wraps them (`Skate3AudioObservation(PlayerAudioObservation)`).
- **Upward deps:**
  - `native_matrix` moves into the sim.
  - `TrainerTuning` becomes a sim-owned `Trainer` struct, mapped from `skate_mods` by the adapter.
  - `crash_context::requested` becomes a `tracing` event with a fixed target, or a `diagnostics` callback.
  - Vehicle `CollisionShape` becomes a sim-neutral `ProxyShape` input.

### 1.4 The ECS-entangled parts (hardest)

- **`physics.rs::advance`** reads 10 resources. Besides the five sim structs it reads:
  - `Vehicles.occupied()`, which pauses the sim while a mod vehicle is driven;
  - `PublishedTickInput`;
  - `Time<Fixed>`: **the sim sets its own cadence** with `cadence.set_timestep(physics.clock.period())`,
    because camera slow-mo requests change the clock;
  - `Option<Performance>` for timing;
  - `MessageWriter<AppExit>`: a failure exits the app.

  The sim API must report `period()` after every tick, and the adapter must reapply it to `Time<Fixed>`.
- **Phase hooks inside one fixed tick:**
  - `session_marker::update` runs between Input and Controls. It reads `ControllerInput` and restores
    checkpoints.
  - multiplayer runs between Controls and Physics (remote proxies).
  - `modding::fixed`, `scoring_hud::advance` and `audio_observation::publish` run after Physics.

  So the sim needs a staged API (`publish_input` / `sample_controls` / `advance`), not just one `tick()`.
- **Run conditions on the sim sets:**
  - `graphics_menu::gameplay_active` (pause, `map_transition.rs:133`);
  - `replay::live` (`replay.rs:127`);
  - engine gating (`engines.rs`).

  These become "the adapter doesn't call `tick`".
- **`map_transition.rs`** reloads `GamePhysics` + `SkaterRuntime` + `PlayerControls` on a worker and swaps
  the resources. It needs `Sim::load(...) -> Sim` to be `Send`. It already is, because these are
  `Resource`s today.
- **Readers of sim internals outside the sim:**
  - `SkaterRuntime` is read by 21 files outside `physics/`: `animation`, `presentation`, `scoring_hud`,
    `customiser`, `custom_models`, `teleport_menu`, `map_transition`, `verification`, `crash_context`,
    `multiplayer` (+`render.rs`), `modding` (4 files), `session_marker`, `xr/link_retail`.
  - `GamePhysics` is read by 14 files.

  This is where the API surface can balloon. See step S4: an `internals` escape hatch for the desktop
  product, and a narrow stable API for the XR adapter.
- **Sets as ordering anchors elsewhere:** `FrameSet::Physics` is used by `scoring_hud`, and
  `SimulationSet::*` by `presentation`, `replay`, `multiplayer`, `modding` and `session_marker`. These stay
  in the adapter (`app.rs` sets move with it). The sim has no sets.

---

## 2. The interface

### 2.1 Rust API, not a C ABI

**Use a Rust crate consumed as a git dependency pinned by `rev`.**

- Both sides are Rust and built together by one `cargo build`. ABI stability buys nothing here because
  the app always compiles the sim from source at a pinned rev. thug2-headless needed a C ABI only because
  it is C++.
- The renderer needs the sim's *data types*, not just a frame stream. `map_render.rs`, the
  `skate_world.rs` render half, `retail_*`, `customiser_parts.rs` and `scoring_hud.rs` read
  `skate_data::skate_map::SkateMap` (meshes, materials, lightmaps), `skate_data::GameAssets`
  (`character_scene`), `apt_scene::Shapes`/`Draw`, `skate_core::camera::CameraFrame` and `NativeMatrix`.
  Flattening those to C structs would be thousands of lines of marshalling for no gain. With Rust, the
  app just depends on `skate3_sim::data` (a re-export of skate-data).
- glam is shared (§1.3), so pose and camera data cross with zero conversion.
- For a contained API, put the stable surface in one module (`skate3_sim::api`), mark output structs
  `#[non_exhaustive]`, and gate the wide internals behind a `internals` cargo feature that only the
  desktop product enables.
- A C ABI only becomes worth it if a non-Rust host ever wants Skate 3. It could be added later as a thin
  `skate3-sim-ffi` crate.

### 2.2 Proposed API (sketch)

```rust
// ---- data / loading (the sim owns all reading of the imported bundle) ----
pub struct Bundle { /* asset root, game.json, GameAssets manifest */ }
impl Bundle {
    pub fn open(asset_root: &Path) -> Result<Bundle, String>;      // validates private/game.json (setup.rs receipts)
    pub fn manifest(&self) -> &data::GameAssets;                   // character_scene etc., for the app's renderer
    pub fn map(&self, path: &Path) -> Result<data::skate_map::SkateMap, String>;
}
pub struct SimConfig { pub difficulty: Difficulty, pub world: WorldChoice /* Flat|Course|Map(SkateMap) */,
                       pub spawn_offset: f32, pub trainer: Trainer, pub hud: bool }

pub struct Sim { /* GamePhysics, SkaterRuntime, PlayerControls, StockGraphs, CameraRuntime,
                    ControllerInput, AudioObservationCursor, Option<hud::Runtime>, events */ }
impl Sim {
    pub fn load(bundle: &Bundle, cfg: &SimConfig) -> Result<Sim, String>;   // Send; run on a worker
    // ---- inputs ----
    pub fn set_pad(&mut self, slot: usize, pad: Option<Pad>);   // Pad = skate_core XboxState {buttons, triggers, left, right}
    pub fn set_view(&mut self, eyes: Option<[[f32; 3]; 3]>);    // today's OffboardView (camera-relative off-board stick)
    pub fn set_world(&mut self, w: &ArGeometry) -> Result<Applied, String>; // room quads/triangles/edges, PlacedSurfaces
                                                                // pieces, rails; Applied::Deferred while grinding
    pub fn teleport(&mut self, transform: [[f32; 4]; 4]) -> Result<(), String>;
    pub fn set_preferences(&mut self, p: Preferences);          // gestures, truck/wheel, difficulty, trainer
    pub fn set_remote_proxies(&mut self, proxies: &[RemoteProxy]); // multiplayer / vehicles
    pub fn spawn(&self) -> Spawn;                               // map spawn: the rig's suggested origin
    // ---- stepping (fixed tick; the host owns the accumulator) ----
    pub fn period(&self) -> Duration;                           // may change after any tick (slow-mo)
    pub fn publish_input(&mut self);                            // ControllerInput -> TickInput
    pub fn sample_controls(&mut self) -> Result<(), SimError>;
    pub fn advance(&mut self) -> Result<TickReport, SimError>;  // frame::advance + post (audio obs, HUD VM)
    pub fn tick(&mut self) -> Result<TickReport, SimError>;     // the three above
    pub fn input(&self) -> &ControllerInput;                    // for session_marker-style hooks between stages
    // ---- outputs ----
    pub fn pose(&self) -> SkaterPose<'_>;   // { generation, root: Mat4, bones: &[Mat4] (native space), discontinuity }
    pub fn bone_names(&self) -> &[String];  // binds the app's GLB joints (animation.rs::for_scene)
    pub fn board(&self) -> BoardPose;       // deck/truck/wheel part transforms (BodyId order)
    pub fn link(&self) -> Link;             // { deck: (pos, forward), grinding }: feeds SkaterLink
    pub fn camera(&self) -> Option<CameraFrame>; // desktop camera; XR ignores it
    pub fn scoring(&self) -> Scoring;       // hud_input() + new/modified/close flags + multiplier_changed + readable name
    pub fn hud(&self) -> Option<Vec<hud::Draw>>; // apt_scene::draw: 1280x720 APT space, texture by name
    pub fn drain_events(&mut self) -> Vec<SimEvent>; // PhysicsEvent (pop, land, bail, state, grind), front-end sound ids
    pub fn drain_audio(&mut self) -> Vec<PlayerAudioObservation>;
    pub fn net_capture(&self) -> NetState;  // plain body/pose arrays; skate-net packing stays in the app
}
pub mod hud {  // loaded separately because it takes ~4.5 s (scoring_hud::xr::start)
    pub struct Assets { pub shapes: Shapes, pub textures: Vec<(String, [u32; 2], Vec<u8>)> /* RGBA8 sRGB */ }
    pub fn load(hud_root: &Path, sim: &Sim) -> Result<(Runtime, Assets), String>;
}
pub struct PlayerAudio { /* AuthoredRuntime + PlayerSound */ }   // see section 4
impl PlayerAudio {
    pub fn prepare(bundle: &Bundle, cache: Option<&Path>) -> Result<PlayerAudio, String>;
    pub fn observe(&mut self, o: &PlayerAudioObservation) -> Result<(), String>;
    pub fn render_block(&mut self) -> Result<&[f32], String>;  // stereo 48 kHz, PCM_FRAMES_PER_BLOCK frames, gain applied
    pub fn paused(o: &PlayerAudioObservation) -> PlayerAudioObservation; // retail's frozen state (forward())
}
```

Compared with THUG2:

| THUG2 | Skate 3 |
|---|---|
| `thug2_step` | `publish_input`/`sample_controls`/`advance` |
| `thug2_set_world` | `set_world` |
| `thug2_set_rails` | folded into `set_world`, because `set_grind_rails` and `set_real_surfaces` must swap together and both are refused mid-grind |
| `thug2_teleport` | `teleport` |
| `thug2_skater_pose` | `pose()` + `bone_names()`. The bones are model-space native matrices; the root is `animation_to_world` |
| `thug2_hud` + `thug2_hud_texture` | `hud()` + `hud::Assets` |
| `thug2_sound_voices` | `PlayerAudio::render_block` (mixed PCM, see section 4) |

Notes:

- **Skater meshes:** unlike THUG2, the sim does **not** export meshes. The skater is a GLB in the imported
  bundle (`GameAssets.character_scene`, `assets.rs:27`, `world.rs:24`) that Bevy's glTF loader already
  reads. The app keeps loading it and binds joints by name (`animation.rs::for_scene`, `present`). The
  same goes for customiser parts and custom models. The sim exports bone names and matrices only.
- **Units and scale:** the sim is in native Y-up metres, which is what `ArWorld` already carries
  (`link_retail::apply_world` passes it straight through). A world-size slider would be adapter-side
  (`to_game`/`from_game`, like `thug2.rs:328-336`). It doesn't exist for Skate 3 today.
- **dt:** fixed tick only. The adapter keeps Bevy's `FixedUpdate` accumulator and sets its timestep from
  `period()`. `presentation.rs`/`replay` interpolation stays adapter-side because it is render-only by its
  own doc comment.
- **Pause and failure:** the adapter decides whether to tick (menus, replay, engine switch, vehicle
  occupied). `SimError` replaces `AppExit::error()`; the desktop adapter can still exit on it.

### 2.3 Data assets: who loads what

- **App** (finds and installs the bundle):
  - `xr/ui/import.rs`: `Download/Skate3` and `/sdcard/Skate3`, zips, copies into the app's files dir.
  - `android.rs` (`use-retail-data`, pushed `private/game.json`).
  - `clean::wanted()` and `setup::asset_root()`'s env/exe-relative search.

  The app hands the sim a path.
- **Sim** (reads every gameplay file under `<root>`):
  - `setup.rs`'s receipt/`game.json` validation becomes `Bundle::open`.
  - `Collections`, `PhysicsSettings`, `StockGraphs`, `PlayerControls`, `CameraRuntime`, maps
    (`.skate`, `skate_world::collision_world`), `StockGameplayConfig`, difficulty, teleport
    destinations.
  - The HUD files (`<root>/private/hud/runtime/trickdisplay.json` + RGBA files; `SKATE_SCORING_HUD_ROOT`).
  - The player audio banks and `audio-pcm-cache` (`PlayerAudioCatalog::from_assets`,
    `private/stock/data/audio/audiofiles.big`).
- **App renderer** (through Bevy's `AssetServer`, using paths and types from the sim's manifest and
  `SkateMap`):
  - character GLB, map meshes/textures/lightmaps, sky/irradiance, customiser parts.
  - The Android "default asset source = Skate 3 root" hack (`app.rs:57-66`) stays an app decision
    (proposal step 4a).

---

## 3. The patched Bevy dependency (`[patch.crates-io]`, `vendor/`)

All three patches are render- or platform-side. **None of the sim uses them, or Bevy at all.**
Documented in `vendor/README.md`:

| Patch | What changes | Who relies on it |
|---|---|---|
| `winit` 0.30.13 | Android: emit `Focused` after `InitWindow`; add `winit::platform::android::pump_lifecycle` to service the looper while loading | `crates/skate-quest/src/lib.rs:46`, `crates/sk8-quest/src/lib.rs:40` (they pump while `lib.rs::run_with` loads Skate 3 on a worker). App-only. The sim's `load` just has to stay `Send`, so it can run on that worker. |
| `bevy_pbr` 0.18.1 | bindless slab reuse (`material_bind_groups.rs`); cached lightmap bind groups (`lightmap/mod.rs`, `render/mesh_bindings.rs`, `render/mesh.rs`); late-preprocess read-only indirect buffer (`gpu_preprocess.rs`, `mesh_preprocess.wgsl`); meshlet cull fix | `retail_render.rs:149` `#[bindless(limit(64))]` `RetailWorldMaterial`; `skate_world.rs:906` `bevy::pbr::Lightmap` (San Vanelona); `graphics_menu.rs:594` `OcclusionCulling` + `DepthPrepass`; `xr/mod.rs:156` occlusion observer; `retail_shader_tests.rs:76,80` `include_str!("../../../vendor/bevy_pbr/...")`; `profiling.rs:457` |
| `bevy_core_pipeline` 0.18.1 | backport of Bevy #22603 conservative depth pyramid (`experimental/mip_generation`) | GPU occlusion culling (graphics menu, XR eyes) |

- `crates/skate-game/build.rs:29-30` lists `vendor/bevy_pbr` and `vendor/bevy_core_pipeline` in
  `rerun-if-changed`. Whichever crate keeps the provenance `build.rs` keeps this. It is app-side.
- **Consequence:** the sim repo has no `[patch]`, no Bevy, no wgpu. The original objection to a separate
  repo ("every consumer must copy the patch") disappears. The patches stay at the app's workspace root.

---

## 4. Audio

### 4.1 Today's chain (Skate 3 tier)

1. `physics/audio_observation.rs::publish` (FixedUpdate, after Physics) builds a `PlayerAudioObservation`
   from the tick. It carries native PhysOut fields (`RetailAudioInputs`), physics events, the trick id
   and surfaces. It is sim.
2. `player_audio.rs::forward` (Update):
   - folds in front-end one-shots (`FrontEndSoundRequest` from `scoring_hud::multiplier_sounds` and
     `session_marker`);
   - repeats the last observation with `paused = true` while `Time<Virtual>` is paused;
   - sends it on an `mpsc` channel.
3. The worker thread `player_audio.rs::run` does the following:
   - `PlayerSound::frame` runs the recovered audio-state bridge, components and patch messages
     (`player_audio/` 18k, `skate-audio-core` `AuthoredRuntime`);
   - `runtime.pump_once()` renders a block;
   - `downmix` + `OUTPUT_GAIN`, then `LivePcm::push`, paced by `sleep` at 187.5 blocks/s.
4. Output: `LivePcm` (a Bevy `Asset` + `Decodable`), played by one `AudioPlayer` (spatial in AR,
   `follow_skater` puts it at `observation.retail.position`).
5. The clean fallback (`player_audio_clean.rs`, Android without banks, or `SKATE_AUDIO=clean`) reads the
   same `PlayerAudioObservation` and plays CC0 samples through `binaural`/`reverb`/`clean_mix`.

### 4.2 Boundary

**Sim side:**
- the observation producer;
- the observation types;
- the whole recovered graph (`player_audio/`, the worker logic of `player_audio.rs`: `prepare`, `run`'s
  per-block body, `DirectMixer`, `EventProducer`, `downmix`, `OUTPUT_GAIN`);
- `frontend::FrontEndSound` (multiplier ids);
- the paused-frame rule;
- bank/XMA decoding in `skate_audio.rs` (`decode`, `decode_bank_sample`, `resolve_ambience`).

**App side:**
- the thread and its pacing;
- `LivePcm`/`StreamPcm` and Bevy audio sources;
- spatial placement;
- the debug `SKATE_AUDIO_PLAY*` startup hooks;
- the clean fallback.

**Don't use THUG2's voices+samples model here.** In Skate 3 the mixing *is* the recovered graph:
selectors, pitch and envelopes, filters, layers and buses. Exporting voices would mean reimplementing
retail-derived DSP in the app, which conflicts with keeping retail-derived code in the gated engine.
The graph's output is a single mix played from one emitter anyway.

The boundary is:
- **Events + observations out of `Sim`.**
- **A mixed-PCM pull from `PlayerAudio`.** The app calls `observe`/`render_block` on its own thread.

If per-sound spatialisation is wanted later, export per-bus stems, not voices.

**One wrinkle:** the clean driver (bound for `skate-clean-audio`, proposal step 3) must not depend on the
sim, because SK8 and the store build use it. The adapter converts `PlayerAudioObservation` +
`PhysicsEvent`s into the neutral `EngineEvent` + motion state (speed, grounded, grinding, surface) that
the clean driver consumes. Today `player_audio_clean.rs::observe` (line 607) reads the Skate 3 types
under `cfg(feature = "retail")`.

---

## 5. Tests

`#[test]` counts (grep).

### 5.1 Move with the sim

**skate-game: about 464.**
- `physics/` 115 + `physics.rs` 1
- `graph_host/` 71
- `player_audio/` 137 + `player_audio.rs` 15
- `scoring_runtime.rs` 8
- `grind_world/` 10
- `input/` 10 (gestures, controllers)
- `camera/` 2
- `src/tests/` 72 (all except `map_transition.rs`'s 2)
- `skate_audio.rs` 9
- `skate_world.rs` ~8 (collision ones)
- `difficulty`, `map_library`, `setup` 4

**Lower crates: 1,690.**
- `skate-core` 789 (the proposal's figure; 644 by a plain grep of `src/`)
- `skate-audio-core` 776
- `skate-audio-formats` 66
- `skate-data` 59

**About 120 of the skate-game sim tests are `#[ignore]` and need the user's converted data:**
- `SKATE3_ASSET_ROOT` (64 sites, mostly `src/tests/*playback*`, `graph_host/tests/`);
- `SKATE_ASSETS` (9);
- `SKATE_MAP_TEST_PATH`, `SKATE_TRANSITION_TEST_ASSETS`.

They are the real regression suite (trick and playback traces). Running them stays a local/distrobox
job. CI can't, because no game data is checked in.

**Fixtures:**
- `maps/format-demo.skate` (repo root): `include_bytes!` in `skate_world.rs` and `tests/map_startup.rs`.
  Copy it into the sim repo.
- `crates/skate-data/tests/fixtures/retail-collision.rwcmset` moves with skate-data. Fix the
  `../../skate-data/...` include in `skate_world.rs`.
- `src/tests/fixtures/texture-codec.*` belongs to `custom_models.rs` and stays in the app.

**Examples:** `examples/trick_reachability.rs` and `examples/landing_sample_levels.rs` use only
skate-core/data/audio-core. Move them.

### 5.2 Stay in the app

**Render and picture tests:**
- `retail_*` 18, incl. `retail_shader_tests.rs` (naga, reads `vendor/bevy_pbr`) and
  `retail_shadow_visibility_tests.rs`;
- `customiser*` 9, `custom_models` 7, `map_render` 5, `skate_world` render ~8, `graphics_menu` 3,
  `scoring_hud` 2 (incl. ignored `a_picture_of_the_hud`, `SK8_HUD_SHOT`), `animation.rs` 2,
  `camera.rs` 1, `render_capacity` 1.

**Adapter/game-mode tests:**
- `replay` 5, `session_marker` 6, `teleport_menu` 1, `tests/map_transition.rs` 2 (builds a Bevy
  `App`), `link_retail` 1.

**Unchanged:**
- desktop product: `modding` 10, `multiplayer` 7, `crash_report` 2, `profiling` 4, `skate-mods` 27,
  `skate-vehicles` 74, `skate-net` 29;
- `xr/` 150, `engines` 4, `thug2*` 10, `clean` 4.

**Add to the sim:** API-level tests with no assets. Build a `Sim` on `Terrain::Flat`/`Course` from a
tiny synthetic fixture where possible. Otherwise add `#[ignore]` + `SKATE3_ASSET_ROOT` tests that tick N
frames and assert pose/HUD/audio-observation shape.

**Baseline:** before S1, record the test list (`cargo test -- --list`). The pre-existing failures are
`clean::menu` trick_paths and `profiling` (proposal step 0). The per-crate totals must add up after each
step.

---

## 6. Migration path

Sizes: S ≤ 1 day, M 2-3 days, L 4-6 days.

**Prerequisites:** proposal steps 0-3 as written: baseline, `skate-engine-api`, explicit `EngineSet`
ownership, and cutting the shared layer's ties to Skate 3 (the `ExternalPad`/`poll_controllers`
ordering, the rig origin, menus, fonts, `skate-clean-audio`). They're independent of this split and make
S5 cheaper. Step 3's `PadInput` is what S5's pad mapping reads. S1-S3 can start in parallel with steps
1-3, because they only move code downward. **Step 5 (`engine-thug2`) is still the rehearsal** for S5.
Steps 4a/4b stay. Step 6 becomes S5. Step 9 becomes S1-S6.

| Step | What | Size |
|---|---|---|
| S1 | Group Skate 3 crates under `crates/skate3/`; empty `skate3-sim` crate | S |
| S2 | Move the leaf sim modules (no upward deps), de-Bevy as they move | M |
| S3 | Move `graph_host/` + `physics/` + player audio worker together | **L** |
| S4 | `Sim` façade + `PlayerAudio`; port the adapters onto it | M |
| S5 | `engine-skate3` adapter + renderer (mirrors `thug2.rs`) | M-L |
| S6 | `git subtree split` to its own repo, git dep pinned by `rev` | S-M |
| S7 | Cleanup: drop re-exports, shrink `internals`, CI guards | S |

### S1: Group the Skate 3 crates (S)

- `git mv` skate-core, skate-data, skate-audio-core and skate-audio-formats under `crates/skate3/`. Add
  `crates/skate3/sim` (`skate3-sim`) with deps `skate-core`, `skate-data`, `skate-audio-*`,
  `glam = "=0.30.10"`, `tracing`, `serde`, `serde_json`. **No bevy.**
- Fix paths:
  - workspace members and `[workspace.dependencies]`;
  - `skate_world.rs`'s `../../skate-data/tests/fixtures/...` include;
  - `build.rs` `rerun-if-changed` (`../skate-core/src` etc.).
- skate-game: `retail` adds `dep:skate3-sim`.
- Verify: builds; test counts unchanged.

### S2: Move the leaf modules (M)

- **Bottom-up**, with no upward `crate::` deps:
  - `scoring_runtime`;
  - `apt_*` + `hud_runtime`;
  - `difficulty`, `map_library`, the `setup` validation half;
  - `skater_animation`, `animation_pose`, `graph_runtime`;
  - `grind_world/` (provider/octree/spline);
  - `input/` gesture + controllers;
  - `camera/` runtime;
  - the `skate_world.rs` collision half;
  - the audio observation types from `skate_audio.rs`;
  - `native_matrix`.
- De-Bevy each one on the way in (§1.3). skate-game re-exports at the old paths
  (`pub(crate) use skate3_sim::scoring_runtime;`), so call sites don't change. Plugins (`GrindGeometryPlugin`,
  `InputPlugin`) stay in skate-game and wrap the moved structs in `Resource` newtypes.
- **Watch:** `pub(crate)`/`pub(super)` must become `pub` on everything that crosses. Orphan rules: Bevy
  trait impls on moved types (`Resource`, `Message`) must become newtypes.
- Verify: test counts per crate sum to the baseline. Desktop `--check-assets` passes.

### S3: Move the sim core (L, riskiest)

- `graph_host/`, `physics/` and the player-audio worker logic + `player_audio/` move **together**. They
  are mutually referential: physics uses `graph_host`, `camera`, `input`, `grind_world`,
  `skate_audio` types; graph_host uses `graph_runtime`.
- Split `physics.rs`:
  - `GamePhysics`, `SimulationExchange`, the loaders, `finish_skater` go to the sim;
  - `PhysicsPlugin`/`advance`/`present` go to the adapter (`skate-game/src/physics_host.rs` for now).
- Same for `controls.rs::sample` and `audio_observation.rs::publish`: the system shells stay, the bodies
  move.
- Replace the `TrainerTuning` dep, the `crash_context` hook, the vehicle `CollisionShape` and climbing's
  `Transform` (§1.3).
- Split `physics/network.rs`: capture (`anchors`, `capture_body`, `capture_pose`, proxy contacts) goes to
  the sim and returns plain arrays. `skate_net` packing/hashing stays in the app, so the sim doesn't
  depend on skate-net and skate-net stays engine-neutral (the proposal's colocated-multiplayer note).
- `src/tests/*` (except `map_transition.rs`) moves with it. Its `#[path = "tests/..."]` mounts inside
  `physics.rs` keep working if the relative layout is kept (`src/physics.rs` + `src/tests/`).
- **Temporary escape hatch:** `skate3_sim::internals` (feature `internals`, `#[doc(hidden)] pub`) exposes
  `GamePhysics`/`SkaterRuntime` fields to the 21 outside readers until S4 gives them accessors. The
  desktop product (`modding`, `multiplayer`, `customiser`, `verification`, `crash_context`) may keep
  using it permanently.
- **Risks:**
  - biggest diff (~75k lines moved);
  - visibility churn across ~400 files;
  - compile time while both crates are hot;
  - `include_bytes!` depth changes;
  - per-tick ordering must be byte-identical.

  Check that with the recorded-playback tests (`tests/recorded_playback.rs`, `SKATE_REPLAY_*`) and a
  desktop session trace before/after (`offboard_audit_trace`, `SKATE_PLAYER_AUDIO` logs).

### S4: The façade (M)

- Add `Sim`, `Bundle`, `hud`, `PlayerAudio` (§2.2) over the moved structs. `lib.rs::load` becomes
  `Bundle::open` + `Sim::load`.
- Port the consumers:
  - `xr/link_retail.rs` (world, teleport, pose, HUD text);
  - the `scoring_hud` `advance` (now inside `Sim::advance`) and `multiplier_sounds` (now a `SimEvent`);
  - the player audio host (`PlayerAudio::observe`/`render_block` on the app's thread);
  - `presentation::capture` (`Sim::pose`, `Sim::camera`);
  - `map_transition` (`Sim::load` on a worker).
- `Time<Fixed>` timestep comes from `Sim::period()`. Run conditions decide whether to call `tick`.
- **Risks:**
  - The staged API must reproduce the session-marker and multiplayer hook positions exactly.
  - The audio paused-frame rule moves sides.
  - `scoring_hud` HUD reset on `CurrentMap.generation` must become `Sim` state.

### S5: `engine-skate3` adapter + renderer (M-L)

- Proposal step 6, but it now holds only the Bevy side:
  - plugin wiring (`app.rs` sets, `add_retail`);
  - pad → `Pad` (from step 3's `PadInput`) and XInput `platform.rs` for desktop;
  - `ArWorld` → `set_world`;
  - the skinned GLB skater (`animation.rs`, `presentation.rs`, `replay`, `world.rs`, `assets.rs`);
  - the HUD drawing (`scoring_hud.rs` + wgsl);
  - audio output;
  - the retail renderer, customiser, `map_render`, `skate_world` render half;
  - `session_marker`, `teleport_menu`, `graphics_menu`.

  About 25k lines. The desktop product goes to `skate3-desktop` (step 7).
- Keep the `src/` layout for `embedded_asset!` paths. `scoring_hud.rs:33,49` uses `env!("CARGO_CRATE_NAME")`.
- Mirror `thug2.rs`: one `install`, `available()` = `Bundle::open` succeeds.
- Verify:
  - headset Skate 3 with the APT panel, sound, AR world and teleport;
  - `SK8_ENGINE_SCRIPT` switching;
  - the `retail_shader_tests` naga run;
  - Windows `skate3rust`.

### S6: Split the repo (S-M)

- `git subtree split --prefix=crates/skate3 -b skate3-sim`, then push to the new private repo, which
  gets its own `[workspace]` `Cargo.toml` and `rust-toolchain`/edition 2024.
- History caveat: subtree split keeps history only from when files sat under `crates/skate3/`, which is
  S1-S3. If the pre-move history of `physics/` etc. matters, use `git filter-repo` with `--path-rename`
  on a clone instead.
- In the app, replace the path deps with
  `skate3-sim = { git = "<url>", rev = "<sha>", optional = true }`.
  - **Local dev:** `[patch."<url>"] skate3-sim = { path = "../skate3-sim/sim" }` (and the same for its
    crates), toggled in a non-committed `.cargo/config.toml` or a commented block.
  - The sim's own `Cargo.lock` doesn't apply in the app. Pin `glam`/`tracing` with `=` in the sim so the
    app resolves the same versions as Bevy.
- **Blocker to check:** CI must be able to fetch the private repo.
  - `release.yml` (Windows) and `scripts/quest/build-apk.sh` need a deploy key or token
    (`CARGO_NET_GIT_FETCH_WITH_CLI=true` + `GH_TOKEN` secret).
  - Today thug2-headless is a sibling *checkout* (CMake path), not a cargo git dep, so this is new
    plumbing.
- The ignored asset-gated tests now run from the sim repo: `SKATE3_ASSET_ROOT=... cargo test --release
  -- --ignored` in the distrobox.

### S7: Cleanup (S)

- Remove the S2 re-exports. Move remaining `internals` users onto API accessors where cheap.
- Add a CI guard: `cargo tree -p sk8-quest | grep -E 'skate3-sim|skate-core'` must be empty.
- Update `docs/engine-packaging-proposal.md`'s effort table.

---

## 7. Biggest risks and blockers

- **S3 is the big move.** It has a mutually referential module group (physics ↔ graph_host ↔ camera ↔
  input ↔ grind ↔ audio types) that can't be split further cheaply. It needs exact per-tick ordering.
  It has ~400 files of visibility churn. Mitigate with S2 first (leaves), playback/trace comparisons, and
  doing it in one sitting.
- **Hook points inside a tick.** session_marker sits between Input and Controls, multiplayer between
  Controls and Physics, and modding/HUD after Physics. A single `tick()` would change behaviour, so the
  staged API is required.
- **The sim-driven timestep** (`Time<Fixed>` set from `physics.clock.period()`) is easy to lose in the
  port, which breaks slow-mo camera behaviour.
- **API ballooning:** 21 outside files read `SkaterRuntime` internals. Keep the XR adapter on the narrow
  API and let the desktop product use `internals`.
- **Clean audio coupling:** `player_audio_clean.rs` consumes Skate 3 types. It must go through a neutral
  event (aligned with proposal step 3) before skate-clean-audio can be engine-neutral.
- **CI access to a private git dep** (S6) is new plumbing. Disk is short on the host (two target dirs
  while both repos are hot), so build release-only in the distrobox.
- **No hard blocker found.** None of the vendored Bevy patches touch the sim, and the load path is
  already Bevy-free and `Send`. The engine WIP must be committed first (proposal step 0).

---

## 8. Progress (S1-S4 done, 2026-11-16)

- **Layout:** `crates/skate3/{skate-core,skate-data,skate-audio-core,skate-audio-formats,sim}`. `skate3-sim`
  is ~90k lines and has no Bevy (`cargo tree -p skate3-sim | grep -i bevy` is empty). Its module paths
  mirror `skate-game/src`, so `crate::physics::...` resolves the same on both sides.
- **Shims left in skate-game (S7 removes them):** `lib.rs` `use skate3_sim::{difficulty, hud_runtime,
  map_library, apt_scene}`; glob re-exports in `physics.rs`, `camera.rs`, `graph_runtime.rs`,
  `grind_world.rs`, `input.rs`, `skate_world.rs`, `skate_audio.rs`; same-named `Deref` newtypes for the
  resources (`GamePhysics`, `SkaterRuntime`, `PlayerControls`, `StockGraphs`, `CameraRuntime`,
  `ControllerInput`, the `AudioObservationCursor`) and the `PlayerAudioObservation` message.
- **Façade (S4):** `skate3_sim::{Bundle, Sim, SimConfig, PlayerAudio}`. Startup loading is `Bundle::open`
  + `Sim::load` + `into_parts`; the fixed-tick systems call the stage functions `Sim` is made of
  (`input::publish`, `physics::controls::sample`, `physics::advance`, `physics::audio_observation::observe`);
  the audio worker renders with `PlayerAudio::render_block`. `map_transition` still loads the parts itself.
- **Still entangled:** the sim depends on `skate-net` (network.rs packing, not split yet); `setup.rs`
  (receipt validation) stays in the app because it reads `customiser_parts`/`android`; `scoring_hud::advance`
  is not folded into `Sim::advance`; `PlayerAudio` is not `Send` (prepare it on its render thread).
- **Verified:** same 2420 test names and results as the baseline, and the 137 asset-gated tests give
  the same per-test outcomes before and after against the converted data (plus the new façade test).
