# Engine packaging: a standalone Skate 3 engine (and SK8, THUG2) behind one engine API

Status: proposal, 2026-11-16. Research only: no code was changed for this document.

Goal (the user's words): *"Now that we've become a modular skate engine consumer, we should refactor out
our skate 3 code into a standalone package (like thug2-headless). Ditto for sk8 core if needed."*

Snapshot audited: `main` at `11a93ca` plus the uncommitted working tree. `engines.rs`, `thug2.rs`,
`thug2/`, `crates/thug2-sys/`, `xr/hand_hud.rs`, `xr/build/parks.rs` and `xr/ui/parks.rs` are untracked
or modified, so the engine system itself isn't committed yet. Commit it before starting any step below.

---

## 1. Current state

### 1.1 Workspace and crates

Workspace members (`Cargo.toml`), with Rust line counts (`src/**/*.rs`):

| Crate | Lines | What it is | Depends on (workspace) |
|---|---:|---|---|
| `skate-game` | 150,353 | **Everything app-side**: the Skate 3 port's Bevy integration, the SK8 adapter, the THUG2 adapter, the XR layer, menus, audio, the desktop game, `engines.rs` | all below (mostly optional) |
| `skate-core` | 90,714 | Skate 3 "recovered calculations", no ECS/IO | none |
| `skate-audio-core` | 61,727 | Skate 3 audio graph ported from the native C++ | none |
| `skate-data` | 7,841 | Skate 3 stock data, manifests, bank storage | skate-core, skate-audio-* |
| `skate-audio-formats` | 3,625 | EA audio containers | none |
| `skate-net` | 2,612 | Snapshot protocol (`SK8NET01`); `skate-core` is used only by its tests | skate-core |
| `skate-vehicles` | 2,524 | Rapier vehicles + mod definitions | none |
| `skate-mods` | 1,182 | Lua package runtime | skate-vehicles |
| `skate-steam-relay` | 511 | Steam relay | skate-net |
| `thug2-sys` | 868 | THUG2 C API + safe wrapper; CMake-builds `../thug2-headless` + `../thug2-decomp` (`links = "thug2_engine"`) | none |
| `skate-lite-sys` | 823 | **SK8 core**: bindings to the clean-room C++ `sk8` engine vendored from skatecore-cpp `clean/` (GPL-3.0-only) | none |
| `roomtone-sys` | 648 | Room light estimation (MIT, vendored) | none |
| `skate-quest` | 452 | GitHub Quest APK shell (`cdylib`, cargo-apk) | `skate-game` features `xr, retail, thug2` |
| `sk8-quest` | 46 | Store Quest APK shell; reuses skate-quest's `src/*.rs` via `#[path]` | `skate-game` features `xr` |

Dependency graph today:

```
skate-quest ──► skate-game[xr,retail,thug2] ──► thug2-sys ──(cmake)──► ../thug2-headless, ../thug2-decomp
sk8-quest   ──► skate-game[xr]               ├─► skate-lite-sys (SK8 core, C++)
                                             ├─► roomtone-sys (xr)
                                             └─► [retail] skate-core, skate-data, skate-audio-core,
                                                 skate-audio-formats, skate-mods, skate-vehicles, skate-net
```

`skate-game` features: `default = [dev-dynamic, retail]`; `retail` = the whole Skate 3 port plus its
crates; `xr` = OpenXR layer; `thug2 = [xr, dep:thug2-sys]`; `xr-window`. Bins: `skate3rust` (desktop
game, built for the Windows release in `.github/workflows/release.yml`), `skate3-audio-check`,
`skate3-multiplayer`, `skate3-custom-models` (all `required-features = ["retail"]`).

**The three engines are packaged unevenly:**

- **THUG2**: the engine is an external package (thug2-headless, own repo, MIT, C API) behind `thug2-sys`.
  The *adapter* (`thug2.rs` 1,269 + `thug2/hud.rs` 663 + `thug2/hud.wgsl`) still lives in `skate-game`.
- **SK8**: the engine is *also* already an external package. `skate-lite-sys` wraps skatecore-cpp's
  `clean/` (vendored by `scripts/cleanroom/sync-lite.sh`, local changes tracked in `LOCAL-CHANGES.md`,
  mirrored at github.com/rhythmerc/skatecore). That's the thug2-headless model already. The adapter
  (`clean/`, 5,375 lines: `sim.rs` 211, `input.rs` 840, `menu.rs` 1,140, `skater.rs` 767,
  `procedural.rs` 899, `view.rs` 397, `xr_menu.rs` 141, `skater_page.rs` 155, …) lives in `skate-game`.
- **Skate 3**: the Bevy-free lower layers are already crates (`skate-core`, `skate-data`,
  `skate-audio-*`), but the engine as a playable thing is roughly **114k lines of `#[cfg(feature =
  "retail")]` modules inside `skate-game`**. There is no Skate 3 package boundary, only a feature flag.

### 1.2 Where the Skate 3 code lives (inside `skate-game/src`)

Everything `lib.rs` gates with `#[cfg(feature = "retail")]`, about 114k of the crate's 150k lines:

- Simulation: `physics/` 36,644 (`physics.rs` 740), `graph_host/` 14,941, `graph_runtime.rs`,
  `scoring_runtime.rs` 1,602, `skate_world.rs` 1,395, `grind_world/` 936, `camera/` 1,857 + `camera.rs`,
  `animation*.rs`, `skater_animation/`, `replay/`, `input.rs` + `input/` 1,409, `difficulty.rs`,
  `map_*`, `world.rs`, `setup.rs`, `teleport_menu.rs`.
- Audio: `player_audio.rs` 1,888 + `player_audio/` 18,149, `skate_audio.rs` 1,270 (also declares
  `skate_audio::clean`, see 1.5).
- Rendering: `retail_*.rs` 3,952 + 8 `.wgsl`, `customiser*.rs` ~2,800 + 2 `.wgsl`, `custom_models.rs`,
  `session_marker/` 877 + `effect.wgsl`, `render_capacity.rs`, `graphics_menu.rs` 914, `presentation.rs`.
- HUD: `scoring_hud.rs` 742 (APT HUD; flat-screen overlay and an XR `HandHud` panel),
  `apt_*.rs` + `hud_runtime.rs` 1,704, `hud_render.wgsl`, `hud_composite.wgsl`.
- Desktop product: `multiplayer.rs` + `multiplayer/` 1,798 (uses `skate-net`), `modding/` 4,244,
  `updater.rs`, `crash_*.rs`, `profiling.rs` 658, `performance.rs`, `verification.rs`, `fps_overlay.rs`.
- Wiring: `app.rs` (`build`, `add_retail`, `FrameSet`, `SimulationSet`), `lib.rs::load/run_retail`,
  `config.rs` (`Config` is retail-only; `XrOptions` is shared).
- XR glue that is Skate 3-only but sits in the XR tree: `xr/link_retail.rs` 197 (`RetailLinkPlugin`),
  `xr/ui/import.rs` 342 (in-headset Skate 3 importer), the "Skate 3 data" page in `xr/ui/pages.rs`,
  `xr::retail_controls()`, `xr::add_retail_link()`; `clean_pack.rs` 137 (retail + Android).
- Tests: `src/tests/` 7,088 lines (74 tests), plus about 600 unit tests across physics/graph_host/
  player_audio/scoring. `examples/landing_sample_levels.rs`, `examples/trick_reachability.rs`.

Only about 19 of 271 `physics/` files and 1 of 97 `graph_host/` files mention `bevy`, and `player_audio/`
none. Most of the simulation is close to Bevy-free; the Bevy-facing part is the plugins, resources
(`GamePhysics`, `SkaterRuntime`, `PlayerControls`) and the rendering.

### 1.3 The shared app layer (always built, ~36k lines)

- `xr/` 23,475: `mod.rs` (`plugins()`, `XrPlugin`), `rig.rs`, `actions.rs`, `touch.rs`, `spawn.rs`,
  `link.rs` (the engine contract, see 1.4), `real_world.rs` 575 (room collision → `ArWorld`),
  `build/` (park builder: `mod.rs` 3,602, `pieces.rs`, `parks.rs` 939, `pool*.rs`, `drawn.rs`, …),
  `ui/` (menus: `mod.rs` 1,211, `pages.rs` 734, `parks.rs` 1,579, `hud.rs` 189, `sound_lab.rs`,
  `pointer.rs`, `import.rs`), `hand_hud.rs` 194, `panel.rs`, occlusion (`occlusion*.rs`,
  `controller_shapes.rs`), `room_light/`, `planes.rs`, `mesh_planes.rs`, `contact_shadow.rs`,
  `spatial_audio.rs`.
- `engines.rs` 632: the engine registry and switching.
- Shared audio, used by Skate 3 *and* SK8: `binaural.rs` 552, `reverb.rs` 930, `clean_mix.rs` 314,
  `clean_synth.rs` 812, `stream_pcm.rs` 135, `player_audio_clean.rs` 1,101 (mounted as
  `skate_audio::clean` by both `skate_audio.rs` and `skate_audio_store.rs`), `clean-audio/` (2.5 MB WAVs,
  `include_bytes!` via `CARGO_MANIFEST_DIR`).
- Two app builders: `app::build` (retail; used whenever Skate 3 data is present) and `clean::build`
  (no Skate 3 data, and the store build). `lib.rs::run_with` picks between them with
  `clean::wanted()`, a function in the **SK8 module** that checks for `assets/private/game.json`.

### 1.4 How engines are integrated: `engines.rs`

- `Engine` is a closed enum `{Sk8, Skate3, Thug2}` with `key()` (`"sk8"/"skate3"/"thug2"`, persisted in
  `engine.txt` and in park files' `engine` field).
- `add_engine(app, engine, available, plugins)`:
  1. Removes `PreStartup/Startup/PostStartup` from `Schedules` (so the engine's startup systems go into
     fresh schedules), snapshots every schedule's `SystemKey`s (`system_keys`), runs `plugins(app)`.
  2. The fresh startup schedules become the engine's own, run on its first `enter` (`Engines.startup`).
  3. Every other schedule: systems whose key wasn't in the snapshot are **inferred** to be the
     engine's. Each is wrapped in `Owned { engine, inner }` (sets the `RUNNING` thread-local during
     `run_unsafe`/`apply_deferred`/`queue_deferred`) and gets a `running(engine)` run condition.
  4. Observers `tag::<Visibility>` and `tag::<PlaybackSettings>` insert `EngineOwned(engine)` on any
     entity whose `Visibility`/`PlaybackSettings` is added while `RUNNING` is set.
- `switch` (in `First`): runs `BeforeSwitch` hooks, `leave` (hide every `EngineOwned` entity of the old
  engine and pause its sinks), `enter` (run deferred startup once; restore `ClearColor` afterwards;
  else restore hidden/paused), writes `EngineSwitched`, saves `engine.txt`.
- Registration sites: `app.rs:130` (Skate 3: `add_retail` + `xr::add_retail_link`),
  `clean/mod.rs:216` (SK8: `add_sk8`), `thug2.rs:111` (THUG2). `clean::add_xr_engines` also
  installs THUG2 and applies `use-retail-data`.

**Where the inference is fragile** (the STARTUP bug is one instance of these):

- *Ownership = "registered during the closure".* Anything added to a schedule inside the closure is the
  engine's, even when it's shared: a Bevy plugin added for the first time there (`Material2dPlugin`,
  `UiMaterialPlugin` in `ScoringHudPlugin`), or a shared helper both engines call. Anything added
  outside isn't, even when it's engine-specific (THUG2's `shoot`, `add_audio_source::<Thug2Pcm>`).
- *The snapshot is easy to get wrong.* The STARTUP bug happened because `before` was taken with the
  shared startup schedules removed, so every shared startup system (menu panel, its camera, the UI
  root) looked new and was wrapped as Skate 3. It's patched by skipping startup labels in the post-loop
  (`engines.rs:238`). `xr/panel.rs::report_on_open` is a diagnostic that was added to chase it.
- *Not covered at all:* systems added in `Plugin::finish/cleanup` (these run in `app.finish()`, after
  `add_engine` returns); the render sub-app (Skate 3's `retail_shadow`, `retail_exposure`,
  `render_capacity`, `performance` add `Render` systems that run whatever the engine); global observers
  added by an engine; resources (all engines share one `World`; nothing is namespaced).
- *Tagging only sees `Visibility`/`PlaybackSettings`.* Entities without them (cameras with no
  `Visibility`, pure data entities, `Screenshot`s) aren't owned. Entities a *shared* system spawns in
  response to an engine aren't owned either.
- *Transition schedules* are detected by `format!("{label:?}").starts_with("OnEnter")` and similar
  string checks.
- *Engine-specific special cases in generic code:* `enter` restores `ClearColor` because engine
  startups overwrite it; `xr::headset_only_rendering` turns off every camera except `XrCamera` and
  `XrPanelCamera`, so engine HUD cameras must carry the *panel's* marker to survive.

### 1.5 Coupling points between the shared layer and the engines

**The de-facto engine contract already exists** (in `xr/link.rs`, `xr/ui`, `engines.rs`), but it's
`pub(crate)` and mixed in with XR internals:

| Direction | Item | Defined in | Used by |
|---|---|---|---|
| in | `ArWorld`, `PieceSurfaces` (room/pieces/rails, game metres, `revision`) | `xr/link.rs` | `link_retail::apply_world`, `clean::sim::apply_ar_world`, `thug2::apply_world` |
| in | `TeleportSkater` message | `xr/link.rs` | all three adapters |
| out | `SkaterLink { deck, grinding, size }` | `xr/link.rs` | spawn, build, link `note_spot`, real_world |
| out | `TrickHud { name, points, line }` | `xr/ui/hud.rs` | written by `link_retail::publish_hud`, `clean::xr_menu::feed_hud`, `thug2::finish_boot`; reset in `link::carry_over` |
| out | `HandHud` component (+ camera) | `xr/hand_hud.rs` | `scoring_hud::xr::finish`, `thug2/hud.rs` |
| out | `UiProvider` pages, `ControlsSheet`, `ParkSettings` | `xr/ui/mod.rs`, `engines.rs` | `clean::skater_page`, `thug2::Thug2Pages`, `xr::add_retail_link` |
| in | `LatestTouch`/`TouchState`, `UiOwnsControllers`, `BuildMode`, `PlaceSkater` | `xr/actions.rs`, `xr/ui`, `xr/build`, `xr/spawn` | `thug2::read_pad`, `clean::input` |
| in | `Rig` (world scale/origin), eye `Transform`s | `xr/rig.rs` | `link_retail::publish_view` (→ `physics::OffboardView`), `clean` |

Adapters reach into XR internals well beyond that table. `thug2.rs`/`thug2/hud.rs` import 24 distinct
`crate::xr::*` items (including `xr::ui::pages::put_skater_ahead`, `XrUi::say`,
`xr::panel::XrPanelCamera`, `xr::ar_settings::HudHand`), and `clean/` imports about 20.

**Shared code that depends on Skate 3 (the direction that blocks extraction):**

- *Pad plumbing:* `xr/actions.rs::read`, `xr/spawn.rs::aim_and_spawn` and `xr/build/mod.rs`
  (`build_controls`, plus a direct `ExternalPad::set` at line ~1934) write `crate::input::ExternalPad` and
  order themselves `.before(crate::input::poll_controllers)`. Those are Skate 3's pad resource and
  system. The store build compiles `input_store.rs` stand-ins (an empty `ExternalPad`, a never-scheduled
  `poll_controllers`) only so the XR layer builds. `xr/touch.rs` re-exports `skate_core::input::xbox::XboxState`
  under `retail` and defines a copy otherwise.
- *Rig origin:* `xr/rig.rs::RigPlugin::build` reads the retail `Config` and calls
  `spawn_ground(config)` (Skate 3 map spawn, `physics::ground::FLOOR_HEIGHT`). In the retail host
  **all three engines** share Skate 3's map spawn as the world origin; in the clean host it's zero.
- *Menus:* `xr/ui/mod.rs` adds `import::ImportPlugin` under `retail`; `xr/ui/pages.rs` has the
  "Skate 3 data" page, Skate 3-specific engine text, and `ENGINE_IDS`/`engine_about` hard-coded for all
  three engines. `xr/mod.rs` picks the OpenXR app name by feature.
- *Fonts:* the shared menus' fonts are `embedded://skate_game/clean/assets/fonts/...`
  (`xr/ui/mod.rs:48-50`), embedded by `clean::embed_font`, so the shared UI depends on the SK8
  module (`XrUiPlugin::build` calls it).
- *Audio:* `xr/real_world.rs` fills `skate_audio::clean::SurfaceHints`; `xr/spatial_audio.rs` uses
  `SpatialSkateAudio` and `reverb::room`; `xr/ui/sound_lab.rs` tunes `clean_mix`/`clean_synth`;
  `XrPlugin` inserts `skate_audio::clean::TuningFile`.

**Skate 3 code that depends on the app/XR layer:**

- `app.rs::build` is *the host app* whenever Skate 3 data exists. It configures `DefaultPlugins`
  (window title, Vulkan, `AssetPlugin.file_path = config.asset_root`, and on Android the **default asset
  source** pointed at Skate 3's asset root), adds `XrPlugin`, and inserts Skate 3 resources *outside*
  `add_engine` (`Config`, `RetailScene`, `AssetManifest`, `StockGraphs`, `GamePhysics`,
  `SkaterRuntime`, `PlayerControls`, `PlayerAudioAssets`). So they're shared, not owned.
- Skate 3 is loaded synchronously at launch (`lib.rs::load`, on a worker thread with the Android pump)
  whether or not it's the engine picked. Data imported during a run needs a restart
  (`pages.rs`: "reopen the app to load it").
- `scoring_hud.rs` (`XrPanelCamera`, `HandHud`, `Config.xr`), `camera.rs:149`, `map_render.rs:111`
  check `config.xr`.

**Shared state and resources (one `World`):**

- `skate_audio::clean` is installed twice: `install` (Skate 3, inside `SkateAudioPlugin`) and
  `install_sk8` (SK8). Both register `CleanPcm`/`ReverbPcm` sources and init `RoomFromWorld`/`SurfaceHints`,
  and each adds its own `setup`, `lab`, `mix`, `follow_skater`. They're gated separately, but they share
  the `CleanAudio` resource.
- Render layers are hand-allocated with no registry: Skate 3 uses 27, 28 (`retail_character`,
  `retail_shadow_geometry`), 29 (`session_marker`) and 31 (`scoring_hud`, `retail_character`); THUG2's HUD
  uses **28, 29, 30**; controller shapes use 20, 21. Overlaps (28, 29) are harmless today only because
  the other engine's entities are hidden.
- Schedule ordering across the boundary: `UiSet` (adapters run `.after(UiSet)`),
  `TransformSystems::Propagate`, `AnimationSystems`, `crate::input::poll_controllers` (above);
  Skate 3's own `FrameSet`/`SimulationSet` live in `app.rs`.

### 1.6 Multiplayer, audio and HUD today

- Multiplayer (`multiplayer.rs`, `skate-net`, `skate-steam-relay`) is desktop Skate 3 only: it reads
  `GamePhysics`/`SkaterRuntime`/`physics::network` and is added in `add_retail`. `skate-net` is itself
  engine-neutral (its `skate-core` use is test-only). `docs/colocated-multiplayer-plan.md` wants
  headset play, which would need an engine-neutral snapshot hook.
- HUD: three layers. The generic corner readout (`xr/ui/hud.rs`, fed by `TrickHud`) hides itself when the
  running engine owns a `HandHud` (it queries `EngineOwned` on `HandHud`). The engine HUDs are Skate 3's
  APT HUD (`scoring_hud::xr`) and THUG2's (`thug2/hud.rs`).

### 1.7 Tests

- Counts (`#[test]`): skate-game 749 (physics 115, graph_host 71, player_audio 137, xr 150, `src/tests` 74,
  engines 4, thug2 4, clean 4, scoring_runtime 8, profiling 4, retail shader tests 4), skate-core 789,
  skate-audio-core 776, skate-vehicles 74, skate-audio-formats 66, skate-data 59, skate-net 29,
  skate-mods 27, roomtone-sys 6, thug2-sys 5, skate-lite-sys 5.
- Engine-switching tests: `engines.rs` tests (4), `thug2.rs` tests (~1117+), `xr/build/parks.rs`
  (~860+), `xr/ui/parks.rs` (~1422+, 1479+); all build mini apps with `add_engine`.
- Picture-style checks: the parks screen render (`xr/ui/parks.rs` ~1350), `THUG2_SHOT`
  (`thug2.rs::shoot`), `SK8_CAPTURE` frame dumps (`clean/mod.rs`), `SK8_ENGINE_SCRIPT` scripted
  switching (`engines.rs::scripted`), `scoring_hud` and `thug2/hud.rs` HUD screenshots.
- Known pre-existing failures (as reported; **not re-run** for this document, since cargo lives in
  the distrobox and disk is short): `clean::menu::tests::trick_paths_read_as_directions`
  (`clean/menu.rs:1121`) and the `profiling` tests (`profiling.rs:534-592`). Record the baseline before
  step 1 so later steps aren't blamed for them.

---

## 2. Desired state

### 2.1 Principles

- **Ownership is declared, not inferred.** An engine says which systems, startup work, render-world
  systems and spawned entities are its own. The host never diffs schedules.
- **The app depends on engines; engines never depend on the app.** Engines depend only on
  `skate-engine-api` (+ Bevy + their own crates). This is what makes "the store build has no Skate 3"
  structural instead of a feature flag.
- **One host builder.** The app builds `DefaultPlugins`, XR, menus and parks once. Engines add
  themselves to it. No engine builds the app.
- **Same model for all three.** THUG2 already comes closest; SK8 and Skate 3 get the same shape.

### 2.2 The engine API (`crates/skate-engine-api`, Bevy only, no OpenXR)

Mostly existing types made public and moved, not invented:

```rust
/// Stable id; the key is what engine.txt and park files store ("sk8", "skate3", "thug2").
pub struct EngineId(pub &'static str);

pub struct EngineInfo {
    pub id: EngineId,
    pub name: &'static str,            // "Skate 3"
    pub about: &'static str,           // the Engine page line
    pub controls: Vec<(String, String)>,
    pub has_own_hud: bool,             // replaces the EngineOwned-on-HandHud query in xr/ui/hud.rs
}

pub trait SkateEngine: Send + Sync + 'static {
    fn info(&self) -> EngineInfo;
    /// Ungated, before DefaultPlugins finish: asset sources, audio sources, material plugins,
    /// embedded assets, UI providers, park settings. Runs whether or not the engine is ever picked.
    fn prepare(&self, app: &mut App) {}
    /// Gated: everything this engine runs. Systems go in EngineSet(id) (the host adds the
    /// run condition and ownership); startup work goes in EngineStartup(id).
    fn build(&self, app: &mut App);
    /// Whether it can be picked now (data present, not failed).
    fn available(&self, world: &World) -> Result<(), String>;
    /// Optional: park-saved settings, and network snapshots.
    fn park_settings(&self) -> Option<ParkSettingsHooks> { None }
    fn net(&self) -> Option<&dyn NetSnapshot> { None }
}
```

**What an engine receives** (resources/messages the host writes, engine only reads):

- `PadInput`: a neutral controller state (today's `TouchState`: sticks, triggers, grips, A/B/X/Y,
  menu) plus `InputHold { menu_or_build: bool, placing: bool, aiming: bool }`. This replaces the engines
  reading `UiOwnsControllers`, `BuildMode`, `PlaceSkater` and `LatestTouch` directly. Skate 3 converts it
  to its `XboxState`/`ExternalPad` *inside its own crate*, so the shared XR layer stops ordering against
  `poll_controllers`.
- `ArWorld` + `PieceSurfaces` (unchanged), `TeleportSkater` (unchanged).
- `WorldFrame { scale, origin }`: today's `Rig.scale`/`world_origin`, read-only. An engine may
  *suggest* an origin (Skate 3: its map spawn). See the open questions.
- `ViewPose`: the eyes' orientation in game space. Skate 3's `publish_view` → `OffboardView` uses it
  for camera-relative off-board steering.
- Time: Bevy `Time<Real>`/`Time<Fixed>` as now, plus the `EnginePaused` the host sets during menus.
- Lifecycle: `EngineSwitched`, `BeforeSwitch` (unchanged), `EngineStartup(id)` schedule.

**What an engine provides:**

- `SkaterLink` each frame (deck pose, grinding, size).
- Its own render entities (skater, board, world dressing), spawned in `EngineSet`/`EngineStartup`, so
  the host owns and hides them. Optionally under an `EngineRoot(id)` entity for new engines.
- `TrickHud` text for the generic readout, and/or `HandHud` panels with an `EngineHudCamera` marker. The
  host's `headset_only_rendering` honours that marker, so HUD cameras no longer need `XrPanelCamera`.
- Gameplay events: an `EngineEvent` message (`Pop`, `Land { speed }`, `Bail`, `Trick { name, points }`,
  `ComboBanked { score }`, `Grind { on }`) for host features (clean audio, stats, future multiplayer
  feedback). Optional; engines with their own audio (Skate 3 retail audio, THUG2 sounds) keep it.
- Audio: engines spawn their own `AudioPlayer`/spatial sinks (owned and paused on switch as now).
  `SurfaceHints` stay host-filled.
- Menu pages: `UiProvider` and its data types (`UiPage`, `Widget`, `Act`, `Outcome`, `PageId`) move into
  the API as plain data. `UiProvider::engine()` goes away because the host knows which engine registered
  the provider. A small `HostUi` command message (`Say(text)`, `PutSkaterAhead`, `OpenPage`) replaces
  calls to `XrUi::say` and `xr::ui::pages::put_skater_ahead`.
- `ParkSettingsHooks` (get/apply JSON), as now.
- `NetSnapshot` (optional): `snapshot(&World) -> Option<Vec<u8>>`, `apply_remote(&mut World, actor,
  &[u8])`, plus a version tag. Only Skate 3 implements it at first (wrapping `physics::network` +
  `skate-net::packed`).

Also in the API crate: `RenderLayerAlloc` (a registry, or at least published constants per engine)
and the shared font asset ids if fonts move there.

### 2.3 Package boundaries

```
crates/
  skate-engine-api/   NEW  ~1-1.5k lines. Bevy. Contract types + EngineSet/EngineStartup/EngineOwned.
  skate-clean-audio/  NEW  binaural, reverb, clean_mix, clean_synth, stream_pcm, player_audio_clean,
                           clean-audio/ WAVs. Used by the app (SK8, sound lab, spatial audio) and Skate 3
                           (its Android fallback). Depends on skate-engine-api only.
  engine-thug2/       NEW  thug2.rs, thug2/hud.rs, hud.wgsl → depends on thug2-sys + api.
  engine-skate3/      NEW  every `retail` module of skate-game, link_retail, scoring_hud, the
                           importer's logic, clean_pack, shaders, tests, examples. Depends on api,
                           skate-clean-audio, skate-core, skate-data, skate-audio-*, skate-mods,
                           skate-vehicles, skate-net.
  skate3-desktop/     NEW (or a bin inside engine-skate3): the non-XR `skate3rust` Windows game,
                           updater/crash report/profiling, its build.rs (provenance, Windows icon).
  skate-game/         The app/host: xr/, engines.rs (registry), menus, parks, config (XrOptions),
                      android, SK8 adapter (clean/). Features `skate3`, `thug2` become optional deps.
  skate-quest/        deps: skate-game[skate3, thug2]
  sk8-quest/          deps: skate-game (no engines beyond SK8)
  (unchanged) skate-lite-sys, thug2-sys, roomtone-sys, skate-core, skate-data, skate-audio-*, ...
```

Optional later grouping: move the Skate 3 crates under `crates/skate3/` (`engine`, `core`, `data`,
`audio-core`, `audio-formats`, `mods`, `vehicles`, `net`) so a future `git subtree split` is one path.

### 2.4 Does SK8 core need extracting?

**No.** SK8 core *is* already the standalone package: the C++ `sk8` engine in skatecore-cpp
(`clean/`), consumed through `skate-lite-sys`, exactly like thug2-headless/`thug2-sys`. What's left in
the app is the adapter (`clean/`, about 5.4k lines: input mapping, procedural skater, menu sheet, glue).

Leave the SK8 adapter in the app for now:

- SK8 is the default engine and the *only* one in the store build, so the app always ships it. A
  separate crate adds a boundary without changing what ships.
- It owns host-ish things that should move *out* of it, not with it: `clean::wanted()` (picks the host
  builder), `embed_font` (the shared UI's fonts), `UiCapture`, the desktop `clean::build`.
- Once those are moved to the host, `clean/` uses only the API and can become `engine-sk8` cheaply
  (step 8, optional). Do that if a second app wants SK8 or if you want the API proven by three
  out-of-crate engines.

One real SK8 debt to fix in the meantime: `LOCAL-CHANGES.md` lists changes made in the vendored copy that
must be carried back to skatecore-cpp before the next `sync-lite.sh`.

### 2.5 Licensing and shipping (noted, not decisive)

- The app is GPL-3.0; `skate-lite-sys` is GPL-3.0-only; roomtone is MIT.
- thug2-headless declares MIT but compiles thug2-decomp code, a fan project with no licence granted, and
  that code is linked into the GPL app binary in the GitHub build. Skate 3's `skate-core`/
  `skate-audio-core` describe themselves as recovered from or ported against the retail game. Both
  argue for keeping each such engine **in its own package that the store build doesn't depend on**.
  With crates instead of `cfg` that's enforced by the dependency graph (`cargo tree -p sk8-quest`
  shows it), not by remembering a feature flag. It doesn't require separate repos.
- No game data ships for either engine. Skate 3 needs the imported bundle (`Download/Skate3`,
  `assets/private/game.json`); THUG2 needs `files/thug2/Data`. The `available()` hook carries that.
- The fork publishes its own GitHub releases (Windows `skate3rust` via `release.yml`, Quest APKs via
  `scripts/quest/build-apk.sh`/`build-store-apk.sh`). Crates don't change what's released, only how it's
  assembled.

---

## 3. Technical proposal: incremental migration

Each step builds and keeps tests green on its own. "Verify" always includes: `cargo test --release
-p skate-game` (and the crates touched), building `sk8-quest` and `skate-quest` APKs, and the switch
smoke test `SK8_ENGINE_SCRIPT="5:thug2,12:skate3,20:sk8"` on headset. Release builds only, given the disk.

Sizes are rough, for one focused developer: S ≤ 1 day, M 2-3 days, L 4-6 days.

### Step 0: Baseline (S)

- Commit the engine work-in-progress (engines.rs, THUG2, parks).
- Record the failing tests (`clean::menu` trick_paths, `profiling`) and fix or `#[ignore]` them with a
  reason, so "tests pass" means something.
- Add an **ownership snapshot test**: build the real headset app (retail + thug2 features, no XR
  session) and log, per engine, the names of the systems `add_engine` wrapped. Assert that no `xr::` or
  `engines::` system is owned by an engine, and that the menu panel camera and `UiRoot` have no
  `EngineOwned` after startup. That pins the current behaviour before anything moves.

### Step 1: Create `skate-engine-api` with the existing contract (M)

- **Moves:** `Engine` (as `EngineId` + the three well-known ids), `EngineOwned`, `EngineSwitched`,
  `BeforeSwitch`, `ParkSettings`, `running()`, `ArWorld`/`PieceSurfaces`/`SkaterLink`/`TeleportSkater`
  (from `xr/link.rs`), `TrickHud` (data only; the drawing stays in `xr/ui/hud.rs`), `HandHud` (the
  component; placement stays in `xr/hand_hud.rs`), `UiProvider` + `UiPage`/`Widget`/`Act`/`Outcome`/
  `PageId`, `ControlsSheet`, `TouchState` → `PadInput`.
- **skate-game** re-exports them at the old paths (`pub(crate) use skate_engine_api::…`), so no call
  site changes yet.
- **engines.rs:** unchanged in logic; types come from the API.
- **Risks:** low. Watch `HandHud` because it depends on `HudHand` (move that enum too), and on Bevy
  feature selection: the API crate should depend on `bevy` with `default-features = false` and only the
  needed sub-crates (`bevy_ecs`, `bevy_app`, `bevy_math`, `bevy_transform`, `bevy_asset`), so it doesn't
  trip the `android-native-activity` vs `GameActivity` feature choice.
- **Verify:** unit tests; nothing should change at runtime.

### Step 2: Make ownership explicit in `engines.rs` (M)

- Add `EngineSet(EngineId)` (a `SystemSet`), configured per schedule with `.run_if(running(id))`, and
  an `EngineStartup(EngineId)` schedule label run on first `enter` instead of stealing
  `PreStartup/Startup/PostStartup`.
- Ownership = membership in `EngineSet(id)`, read from the schedule graph's hierarchy, plus the
  `Owned` wrapper for spawn tagging (kept: it works, and THUG2 relies on it). No more `SystemKey`
  diffing, no startup-label skipping, no `is_transition` string checks.
- Add `SkateEngine::prepare` and call it for every carried engine **before** any `build`, outside
  ownership. Move the things that must stay ungated there: `Material2dPlugin::<HudMaterial>` and
  `UiMaterialPlugin::<HudComposite>` (scoring_hud), `add_audio_source` calls (`Thug2Pcm`, `CleanPcm`,
  `ReverbPcm`), `embedded_asset!` registrations, UI providers, `ControlsSheet`, `ParkSettings`.
- Keep `add_engine(closure)` as a **compatibility shim** for Skate 3 only: it wraps whatever the
  closure registers into `EngineSet(skate3)` (the old diff, but now writing set membership), and it's
  annotated as legacy. THUG2 and SK8 move to `in_set(EngineSet(..))` / `EngineStartup` directly; their
  plugin bodies are small (`thug2::install`, `clean::add_sk8`).
- Render sub-app: add a `RenderEngineGate` resource extracted from `Engines.current`. Skate 3's
  `Render` systems (`retail_shadow`, `retail_exposure`, `render_capacity`, `performance`) get a
  `run_if` on it.
- Replace `enter`'s `ClearColor` restore with a host rule: engines don't set `ClearColor` in XR (the
  host owns it). Skate 3's startup that sets it is guarded by "not XR".
- Add `EngineHudCamera` and make `headset_only_rendering` honour it. HUD cameras stop using
  `XrPanelCamera`.
- **Risks (Bevy):** set run conditions are evaluated once per set per schedule run, which matches the
  current semantics. Systems in both a shared set (`UiSet`) and an engine set are fine. Systems added
  with `.after(SomeEngineSystem)` from shared code would now silently depend on a gated system, so grep
  for them. Startup systems in `EngineStartup` run via `schedule.run(world)` as today, so the
  `apply_deferred` behaviour is unchanged.
- **Verify:** step-0 ownership test (now asserting set membership); `engines.rs` tests rewritten
  against `EngineSet`; on headset: menu, parks screen and panel persist across SK8 → THUG2 → Skate 3 →
  SK8; HUDs (APT panel, THUG2 panel) appear only with their engine; audio pauses and resumes.

### Step 3: Cut the shared layer's dependencies on Skate 3 and SK8 modules (M-L)

- **Pad:** shared XR systems (`actions::read`, `spawn::aim_and_spawn`, `build::build_controls` and the
  direct `ExternalPad::set` near `build/mod.rs:1934`) write `PadInput` + `InputHold` in a
  `HostInputSet` (PreUpdate) and drop `.before(crate::input::poll_controllers)`. Skate 3 adds a system
  `pad_from_host.after(HostInputSet).before(poll_controllers)` in its own set that fills `ExternalPad`.
  Delete `input_store.rs` and the `XboxState` copy in `xr/touch.rs`.
- **Rig:** `RigPlugin` stops reading `Config`. The origin comes from `WorldFrame`, set by the host at
  launch from the launch engine's `suggested_origin()` (Skate 3: map spawn; others: zero). That's the
  current behaviour, made explicit (see the open questions).
- **Menus:** the "Skate 3 data" page and `import::ImportPlugin` become Skate 3's `UiProvider`
  (registered in `prepare`, shown even when Skate 3 isn't running, since import happens before it can
  run). `ENGINE_IDS`/`engine_about` come from `EngineInfo`. The OpenXR app name comes from the shell
  crate (an `XrOptions` field) instead of `cfg(feature = "retail")`.
- **Fonts:** move `Montserrat-Italic.ttf`/`Inter.ttf` + OFL texts to the app's `xr/ui/assets/` (or the
  API crate) and embed them in `XrUiPlugin`; SK8's menu uses the same constants. This removes
  `clean::embed_font` from the shared path.
- **Audio:** extract `skate-clean-audio` (binaural, reverb, clean_mix, clean_synth, stream_pcm,
  player_audio_clean, `clean-audio/`). Its install is split into `prepare` (sources, shared resources:
  once) and a per-engine driver feeding it (`observe` for Skate 3, `observe_sk8` for SK8).
- **Hooks:** the THUG2 and SK8 adapters replace `XrUi::say`/`put_skater_ahead` with `HostUi` messages,
  and `UiOwnsControllers`/`BuildMode`/`PlaceSkater` reads with `InputHold`.
- **Risks:** pad latency or order changes (check that a flick still registers on the same frame); the
  importer page must work while SK8 runs. Paths: `include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),
  "/clean-audio/..."))` moves with the new crate, so the folder moves too.
- **Verify:** `xr/` must compile with `retail` *off* and without `input_store.rs` (that's the
  store build). Run the sound lab page, spatial audio and THUG2 pad mapping on headset.

### Step 4a: One host builder (M, highest-risk step)

- The app owns `run_with`: build `DefaultPlugins` once (the `clean::build` shape: Vulkan/Metal,
  winit settings, XR wrap). Engines are created from the shell's feature set, `prepare`d, then
  `build`. `clean::wanted()` and `app::build`'s XR branch go away.
- Skate 3 keeps loading at launch for now: `engine_skate3::load()` runs on the loader thread (same
  `pump` mechanism) when data is present, and its result is handed to `Skate3Engine { loaded }`.
- Skate 3's resources (`Config`, `GamePhysics`, `SkaterRuntime`, `StockGraphs`, `AssetManifest`,
  `RetailScene`, `PlayerControls`, `PlayerAudioAssets`) are inserted by its `build`, so they're the
  engine's.
- **Asset sources** (the main Bevy hazard): `register_asset_source` must happen **before**
  `AssetPlugin` is built. `prepare` must therefore run before `add_plugins(DefaultPlugins)`, or the
  host passes a pre-plugin hook. Skate 3 currently replaces the *default* source on Android with its
  asset root. Better: register a named `skate3://` source and keep the default source the app's.
  That touches about 25 `AssetServer::load` call sites in Skate 3 code (and `custom_models`' and
  `mods` sources). If that's too much churn for this step, keep "Skate 3 present ⇒ default source =
  its root" as a host decision and record it as debt.
- The desktop non-XR `skate3rust` path keeps its own builder (it doesn't need the host or XR) and
  moves to `skate3-desktop` in step 7.
- **Risks:** Android load timing and ANR (the pump loop must stay); window title and Vulkan flags
  (`InstanceFlags::empty()` workaround); `WinitSettings`; the `RigPlugin` origin. Desktop `--xr` must
  still work for both data and no-data runs.
- **Verify:** headset with and without Skate 3 data; store APK; `--check-assets`; desktop
  `skate3rust` unchanged.

### Step 4b (optional): Load Skate 3 lazily (M)

- Skate 3's `load()` runs in `EngineStartup(skate3)` on a worker (THUG2's `boot` pattern:
  `Phase::Booting(JoinHandle)` + "Starting Skate 3..." in `TrickHud`). `available()` = data present.
- Gains: launch no longer pays for Skate 3 when another engine is picked; import works without
  restart. Cost: the first switch to Skate 3 takes seconds; `scoring_hud::xr::start` already does the
  same thing for the HUD.

### Step 5: Move THUG2 into `engine-thug2` (S-M)

- **Moves:** `thug2.rs` → `crates/engine-thug2/src/lib.rs`, `thug2/hud.rs` → `src/hud.rs`,
  `thug2/hud.wgsl` → `src/hud.wgsl`. Depends on `thug2-sys`, `skate-engine-api`, bevy.
- **Embedded paths:** `hud.rs` uses `concat!("embedded://", env!("CARGO_CRATE_NAME"),
  "/thug2/hud.wgsl")`. The crate name updates itself (`skate_game` → `engine_thug2`), but the
  **`/thug2/` part is the file's path under `src/`** and must become `/hud.wgsl` to match
  `embedded_asset!(app, "hud.wgsl")` from `src/hud.rs`. A mismatch doesn't fail to compile; the shader
  is just missing at runtime (a magenta or invisible HUD). Add a test that loads every `ShaderRef`
  path through an `AssetServer` with `EmbeddedAssetRegistry`, or at least asserts that the path strings
  equal `embedded_path!` output.
- `stream_pcm` comes from `skate-clean-audio`. The tests (`thug2.rs` ~1117+) move with it and use the
  API's `EngineSet`.
- `skate-game` feature `thug2 = ["xr", "dep:engine-thug2"]`. `skate-quest` unchanged.
- **Risks:** Android CMake build through cargo-apk (`ANDROID_NDK_*`, `THUG2_CXX`, sibling checkouts):
  unchanged, but run `build-apk.sh` once. `links = "thug2_engine"` stays on `thug2-sys`.
- **Verify:** `THUG2_SHOT=… THUG2_SHOT_AFTER=2` picture before and after; on-headset THUG2 HUD panel,
  sounds, world size/speed pages, park settings round-trip.

### Step 6: Move Skate 3 into `engine-skate3` (L, mostly mechanical)

- **Moves:** every `retail` module in `lib.rs`, `xr/link_retail.rs`, `scoring_hud.rs`, the importer
  logic (`xr/ui/import.rs`; it's a Skate 3 `UiProvider` after step 3), `clean_pack.rs`, all `.wgsl`
  next to them, `src/tests/`, `examples/`. `skate-game` loses `retail` and gains
  `skate3 = ["dep:engine-skate3"]`.
- **Keep the `src/` layout identical** (`src/physics/...`, `src/scoring_hud.rs`, …) so every
  `embedded_asset!`/`embedded_path!` relative path stays valid. Only the crate segment changes, and
  `concat!(..., env!("CARGO_CRATE_NAME"), ...)` (`scoring_hud.rs:33,49`) and `embedded_path!`
  (`retail_render.rs`) handle that. Grep for hard-coded `embedded://skate_game/` (today:
  `xr/ui/mod.rs` fonts, `clean/menu.rs`, `clean/skater.rs`, `xr/build/mod.rs` occluder; none of them in
  Skate 3 code) and keep it that way.
- **Relative `include_bytes!`/`include_str!` paths break with directory depth:**
  `skate_world.rs` (`../../../maps/format-demo.skate`, `../../skate-data/tests/fixtures/
  retail-collision.rwcmset`), `tests/map_startup.rs`, `map_render.rs`, `retail_shadow_geometry.rs`,
  `physics/offboard/contact_toolkit/tests.rs`, `retail_shader_tests.rs` (`../../../vendor/bevy_pbr/...`).
  If the crate sits at `crates/engine-skate3/` (same depth as `skate-game`) they keep working.
  Switch them to `concat!(env!("CARGO_MANIFEST_DIR"), "/../..")` anyway.
- **build.rs:** `SKATE_BUILD_ID`/`SKATE_RELEASE_REVISION`/`SKATE_RELEASE_BUILD` are read by
  `crash_report.rs`, `profiling.rs` and `updater.rs`. Copy the provenance part of
  `skate-game/build.rs` into the crate that holds them (engine-skate3 or skate3-desktop), and fix its
  `rerun-if-changed` relative paths.
- **Bevy hazards:** `TypePath` strings of moved assets/materials change (`skate_game::…` →
  `engine_skate3::…`). Harmless unless persisted; nothing in the repo serialises reflected types today.
  Log targets change too (check any `RUST_LOG` filters people use, and
  `tools/asset_pipeline/names.txt` mentions `skate_game::riding_trace`). Plugin uniqueness: if both
  the app and the engine add the same Bevy plugin (e.g. `Material2dPlugin` of a shared type), Bevy
  panics. `prepare` is where shared plugins are added, guarded by `is_plugin_added`.
- **Desktop multiplayer:** stays inside engine-skate3/skate3-desktop. Headset multiplayer would go
  through `NetSnapshot` later.
- **Verify:** the full skate-game test suite (now split across both crates; test counts should add
  up to the step-0 baseline), `retail_shader_tests` (naga validation of the moved shaders),
  `--check-assets` on desktop with imported data, the Windows `Build-Release.ps1` in CI, headset
  Skate 3 with the APT HUD panel.

### Step 7: Desktop Skate 3 as its own crate (S-M)

- `skate3-desktop` holds the `skate3rust` binary, `audio-check`, `multiplayer`, `custom-models`
  bins, `crash_report_ui.ps1`, the Windows `.rc`/icon part of `build.rs`, the `--clean` desktop SK8
  path (or that stays in skate-game's `main.rs`).
- Update `release.yml`/`scripts/Build-Release.ps1` and `BUILD.bat`/`PLAY*.bat` to `-p skate3-desktop`.
- **Verify:** the Windows CI artefact name (`skate3rust-windows-x64.zip`) and updater protocol tests
  (`tools/test_update*.py`) unchanged.

### Step 8 (optional): `engine-sk8` (S-M)

- Only after steps 2-3 leave `clean/` API-only. Moves `clean/` + `assets/skater.glb` (its hard-coded
  `embedded://skate_game/clean/assets/skater.glb` and `HUD_FONT` strings must change). Keep
  `skate-lite-sys` as the core.
- Worth doing if you want all three engines to prove the API, or a second host wants SK8.

### Step 9 (later, optional): A headless Skate 3 sim, and/or a separate repo

- A Bevy-free `skate3-sim` (most of `physics/`, `graph_host/`, `player_audio/`, `scoring_runtime`)
  with a small stepping API (pad in, skater pose and events out), the thug2-headless analogue. Then
  `engine-skate3` is the Bevy adapter plus rendering. This is the point at which a separate repo
  starts to make sense.

### Workspace crate or separate repo for Skate 3?

> **Decided (2026-11-16): separate repo, THUG2's shape.** Skate 3 becomes a Bevy-free simulation in its
> own repo, like thug2-headless; the app hosts all rendering (skinned skater, HUD drawing, audio output)
> through an adapter like `thug2.rs`. The objections below apply to splitting out a *Bevy plugin bundle*;
> they mostly fall away for a Bevy-free sim (rendering, and so the vendored `bevy_pbr`/`bevy_core_pipeline`
> patches, stay in the app). Step 9 becomes the target; the sim boundary is carved inside this workspace
> first, then split out with `git subtree split`. Scoping study: `docs/skate3-sim-split.md`.
>
> The original analysis follows for reference.

**Recommendation: workspace crate(s) now; consider a repo split only after step 9.**

- thug2-headless works as a separate repo because it is **engine-agnostic C++ behind a stable C API**.
  Skate 3's engine today is a Bevy plugin bundle. Out of tree it would have to track the app's exact
  Bevy (`=0.18.1`), wgpu (`=27.0.1`) and, above all, the **vendored patches** (`[patch.crates-io]`
  `bevy_pbr`, `bevy_core_pipeline`, `winit` in `vendor/`). `[patch]` only applies at the workspace root,
  so every consumer would have to copy it.
- The API will churn during steps 1-6; a cross-repo API means lock-step PRs.
- About 1,500 Skate 3 tests and the shader tests read paths in this repo (`vendor/bevy_pbr`, `maps/`,
  `skate-data` fixtures).
- Both release pipelines (Windows, Quest APK) are built from this repo.
- What you lose by staying in the workspace (a hard licensing or provenance wall) you mostly get from
  the crate boundary. `sk8-quest`'s dependency tree contains no Skate 3 or THUG2 crate, and that's
  checkable in CI (`cargo tree -p sk8-quest | grep -E 'engine-skate3|thug2'` must be empty). Grouping
  the Skate 3 crates under `crates/skate3/` keeps a later `git subtree split` cheap.

### Effort summary

| Step | What | Size |
|---|---|---|
| 0 | Baseline, ownership snapshot test | S |
| 1 | `skate-engine-api` with existing contract types | M |
| 2 | Explicit ownership: `EngineSet`, `EngineStartup`, `prepare`, render gating | M |
| 3 | Shared layer free of Skate 3/SK8 modules (pad, rig, menus, fonts, clean audio crate) | M-L |
| 4a | One host builder | M |
| 4b | Lazy Skate 3 load (optional) | M |
| 5 | `engine-thug2` crate | S-M |
| 6 | `engine-skate3` crate | L |
| 7 | `skate3-desktop` crate + CI | S-M |
| 8 | `engine-sk8` crate (optional) | S-M |
| 9 | Headless Skate 3 sim / repo split (optional, later) | L+ |

Steps 1-3 are worth doing even if nothing moves crates: they fix the class of bug behind the
vanishing-menu issue. Step 5 is a cheap rehearsal for step 6.

---

## 4. Open questions (yours to decide)

1. **Lazy Skate 3 (4b)?** Load only when picked (faster launch, import without restart, a few seconds'
   wait on first switch), or keep loading at launch?
2. **The desktop Skate 3 game** (`skate3rust` Windows release, multiplayer, customiser, Lua mods,
   updater): still a product? If yes, `skate3-desktop` crate (recommended) or bins inside
   `engine-skate3`?
3. **World origin.** Today, with Skate 3 data, every engine uses Skate 3's map spawn as the world origin;
   without it, zero. Keep "the launch engine decides", or one fixed origin for all?
4. **Engine identity:** keep a closed set of three known engines (simple, exhaustive matches in menus),
   or an open registry keyed by `EngineId` (needed only if third-party engines are wanted)? Park
   files store keys either way.
5. **Separate repo for Skate 3:** is the crate boundary enough for your licensing and provenance comfort,
   or do you want the retail-derived code out of this repo regardless of the cost (section 3)?
6. **SK8 adapter:** leave `clean/` in the app (recommended) or extract `engine-sk8` for symmetry?
7. **Headset multiplayer:** should the API's `NetSnapshot` be designed now (engine-neutral pose
   snapshots, per `colocated-multiplayer-plan.md`), or left for when multiplayer comes to the headset?
8. **Naming:** `engine-skate3` / `engine-thug2` / `skate-engine-api`, and whether `skate-game` is
   renamed (e.g. `skate-app`). Renaming changes `embedded://skate_game/...` paths and log targets.
9. **Generic trick readout:** keep it as a fallback under engines that have their own HUD, or make
   each engine choose (`EngineInfo.has_own_hud`)?
