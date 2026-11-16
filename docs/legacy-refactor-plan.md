# Desktop-era modules in the headset's Skate 3 path: removal plan

Status: done (passes 1-6, 2026-11-16); kept as the record. Originally: plan, 2026-11-16. It follows the first cleanup pass, which removed modding and vehicles,
multiplayer and skate-net, the updater, crash reporting, profiling, session markers, the customiser menu,
the teleport menu, XInput and desktop setup. Line numbers are as of `198b719`.

## How Skate 3 runs on the headset

- `app::build` registers Skate 3 as
  `engines::add_engine(Engine::Skate3, …, |app| { add_retail(app); xr::add_retail_link(app) })`. Every
  plugin under `add_retail` is registered on the headset.
- Its per-frame systems in `SKATE3` run only while it is the current engine.
- Its `Startup` and `PostStartup` systems run once, on Skate 3's **first turn**. That turn can come in the
  middle of a session, after a switch from SK8 or THUG2.
- RenderApp systems are not engine-gated.
- `FixedUpdate` `SKATE3` pauses on `app::Hold` (from `PadHeld::pause`). `Time<Virtual>` is never paused on
  purpose.
- The desktop `--clean` harness (SK8, Roomtone `SK8_ROOM_LIGHT`) uses none of the modules below. Only the
  retail desktop run (`skate3rust` without `--clean`) is affected by removing them.

## Latent headset bugs these passes fix

- **`graphics_menu::setup` (PostStartup) retargets every `Camera3d` to an offscreen image, with order -1.**
  On a switch into Skate 3 in the middle of a session, that catches SK8's and THUG2's cameras and the
  **XR eye cameras** (`XrCamera` requires `Camera3d`). bevy_mod_openxr restores their render target every
  frame, but nothing restores the order.
- **`graphics_menu::interact`:** Escape on a Bluetooth keyboard toggles an invisible menu that pauses
  `Time<Virtual>`. That freezes the shared XR layer.
- **Replay can be entered:** left menu + X (`BACK`, listed as "Back" on the controls sheet) enters
  instant replay with an invisible panel. The skater freezes and plays the last 30 s; B exits.
  `presentation::capture` also clones about 10 MB of snapshots into its 30 s window every tick.

## Passes

Run `cargo test -p skate-game` and both APK builds after each pass. Before building, grep for removed
resource names: a `Res<X>` left in one of Skate 3's systems panics at runtime on its first turn, not at
compile time.

### Pass 1: diagnostics (S)

| Module | Plan |
|---|---|
| `fps_overlay.rs` | Delete it. It spawns an invisible UI node and runs every frame. |
| `performance.rs` | Delete it. It is inert unless `SKATE_PERF_REPORT` is set. Drop its `Option<ResMut<Performance>>` from `physics.rs:152-171`. |
| `render_capacity.rs` | Delete it. It is a RenderApp system that runs for every engine, written for growing desktop maps. |
| `verification.rs` | Delete it, with `--verify` and `verification_capture` in config. Keep `FrameSet::Verification` as an anchor, or drop the variant and `camera.rs:29`'s `.before` on it. |

**Headset check:** all three engines render the same, logcat shows no wgpu validation errors, and frame
time is the same or better.

**Done (2026-11-16):** the four modules are deleted, with `--verify`, `verification_capture` and the
`FrameSet::Verification` variant (and camera's `.before` on it); physics no longer times itself.

### Pass 2: replay, parked (S)

**Decided (2026-11-16):** keep the replay code for a later headset feature. For now:
- remove the way in, the `SELECT`/`BACK` chord in `replay::controls`;
- stop `presentation::capture` recording snapshots every tick.

Replay stays inert until it gets a headset UI.

**Headset check:** smooth interpolation, the same frozen pose while held, and left menu + X no longer
freezes the skater.

**Done (2026-11-16):** `replay::controls` keeps only B to leave (View and F6 no longer enter), and
`presentation::capture` no longer records. The module is marked parked; its tests call `enter` directly.

### Pass 3: graphics_menu + map_transition together (M)

The two modules reference each other: map_transition calls `resource_mut::<Menu>()`, and `interact` and
`labels` take `MapTransition`. They have to go in the same pass.

1. **`graphics_menu.rs`:**
   - Delete it.
   - Delete `gameplay_active`. Each use is in `FixedUpdate` (already `Hold`-gated) or is always true on the
     headset:
     - `input.rs`: drop the run condition, and call `publish(&mut input, true)`.
     - Do **not** gate `poll_controllers` on `Hold`.
   - Drop scoring_hud's `.after(PresentationSetup)`.
   - Remove `start_paused`.
   - Fix the comment on link_retail's Start mask.
2. **`map_transition.rs` and `tests/map_transition.rs`:**
   - Delete both, with `MapTransitionPlugin` and `MapTransitionSet`.
   - Delete the `world_changed` reloads in retail_exposure, retail_character and retail_render.
   - Delete `MapAssets::retire` (and `publish` stops inserting `MapAssets`).
   - Delete `camera::set_world_environment`.
3. **`map_render`:** move `advance_day` and `position_celestial_bodies` out of graphics_menu's
   registration. They take `Res<Menu>`, so they must not survive it. Delete them, or gate them on
   `DayEnvironment`.
4. **`xr/mod.rs`:** after checking that the eyes carry no `OcclusionCulling`, delete
   `no_eye_occlusion_culling`.

**Headset check:**
- Launch straight into Skate 3, and also switch SK8 → Skate 3 → THUG2 → Skate 3 in the middle of a session.
  The eyes render, there are no camera-order warnings, and passthrough stays clear.
- Pause works on menu and build mode.
- The XR menus keep animating while Skate 3 is paused.
- No "Resource does not exist" panics.
- The room light still fades the AR light.

**Desktop:** `--clean` is unchanged. The retail run (then `--test-world`, a flag since removed) renders straight to the window, without
the Escape menu.

**Done (2026-11-16), in one commit with pass 5** (map_transition and the daylight code only served maps):
- graphics_menu, map_transition and its test are deleted, with `gameplay_active`, `MenuInput`,
  `PresentationSetup`, `start_paused`, `WorldChanged`/`world_changed` and the reload systems,
  `MapAssets`, `set_world_environment`, `advance_day`, `position_celestial_bodies` and
  `no_eye_occlusion_culling`.
- **Eyes and occlusion culling, checked first:** the only code that inserted `OcclusionCulling` or
  `DepthPrepass` was `graphics_menu::apply` (on its first run, on every `Camera3d`). Nothing in the app,
  bevy_mod_xr/openxr 0.5 or the vendored bevy_pbr/core_pipeline does (core_3d only mirrors `DepthPrepass`
  into the render world). So no eye gets it, and the observer had nothing left to strip.
- `apply` also set the eyes' `Msaa` on Skate 3's first turn (to the saved setting, default 4). The eyes now
  keep Bevy's default, `Sample4`.
- **Moved in from pass 4:** `CurrentMap` was only inserted by map_transition, so scoring_hud's `reset`,
  `Hud.generation` and its `Res<CurrentMap>` (in `advance` and `xr::finish`) went here, along with
  `.run_if(gameplay_active)` and `.after(PresentationSetup)`.

### Pass 4: reduce scoring_hud (S-M); keep it, it is live

**What the headset uses:**
- `xr::start`/`xr::finish` load `assets/private/hud` and show it as the hand panel (`EngineHudCamera` +
  `HandHud`).
- `advance`, `render` and `multiplier_sounds` run. The sounds play with or without the HUD.
- Without `private/hud`, the corner `TrickHud` readout is the fallback.

**Plan:**
1. Delete the dead flat path: `setup`, `resize_target`/`resize_image`, `HudComposite`,
   `hud_composite.wgsl`, `UiMaterialPlugin`, and the `composite`/`rebind_after_resize`/`fixed` fields.
2. Delete `reset` and `Hud.generation`. That drops `Res<CurrentMap>` from `advance` and `xr::finish`.
3. Drop `.run_if(gameplay_active)`.
4. Make sure `hud_render.wgsl` is still embedded.

**Headset check:**
- With `private/hud`, the hand panel shows tricks and score by the chosen hand, and the multiplier sounds
  play.
- Without it, the corner readout shows instead.
- Switching engines hides and restores the panel.

**Done (2026-11-16):** the flat path is deleted (`setup`, `resize_target`/`resize_image`,
`HudComposite`, `hud_composite.wgsl`, `UiMaterialPlugin`, and the `target`/`composite`/
`rebind_after_resize`/`fixed` fields, with the resize test). Steps 2 and 3 were done in passes 3+5.
`hud_render.wgsl` is still embedded (`embed_render_shader`). The flat desktop run has no HUD.

### Pass 5 (decided: do it, L): Skate 3 without maps, the AR course only

**Decided (2026-11-16):** retail Skate 3 maps are out of scope. The app is a generic runtime that brings
game engines into custom stages for supported modes.

**On the headset:** there are no maps. Only `assets/` is pushed or imported, so `config.map` is `None`.
`world.rs` uses `map_render` only to spawn the test world without a floor: one `DirectionalLight` at
11,000 lux and ambient at 350. The room light fades both, so they must stay exactly as they are.

**Plan:**
1. Inline that spawn in world.rs.
2. Delete:
   - the map branch of map_render: `custom_lighting`, `celestial_bodies`, `DayEnvironment`, `advance_day`;
   - `retail_backdrop.rs`, `retail_sky.rs`, `skate_world::spawn` and `retail_shadow_geometry`.
3. Remove from config: `--map`, `default_map`, `map`, `map_path` and `map_fingerprint`. The branches that
   depend on them (`RetailScene`, `WorldOrigin`) become constants.

**Headset check:**
- The skater spawns at the same height.
- The lighting is the same with the room light on and off.
- AR collision and rails still rebuild.

**Done (2026-11-16), with pass 3:**
- `world.rs` spawns the course itself, in the same Startup system: `ClearColor` and ambient 350 are
  inserted at build and again at Startup, as before, plus one `DirectionalLight` (11,000 lux, shadows,
  from (4, 7, 4) at the origin, default cascades). The desktop also gets the four floor surfaces. A test
  pins all of this.
- **Deleted:** map_render (all of it: the staging and publishing existed only for maps), skate_world,
  retail_shadow_geometry, retail_backdrop, retail_sky, and the retail world material (`retail_world`,
  `retail_depth`, `retail_material_bindings`, `retail_sky` wgsl), `retail_shadow_visibility` and its
  tests, and `retail_shader_tests` (they validated the world shaders). `retail_render.rs` keeps only what
  pass 6 removes.
- **Config:** `--map`, `default_map`, `map`, `map_path` and `map_fingerprint` are gone. `--test-world` is
  accepted and does nothing (since removed). `RetailScene` is gone: no Hdr/tone on the camera. `WorldOrigin` is always
  `FLOOR_HEIGHT`, `GrindGeometry` is always the test course's, and the sim gets `map: None`.
- The map-only tails of `retail_character::load` and `retail_exposure::load` went too, which left those
  modules inert until pass 6.

### Pass 6 (decided 2026-11-16): strip the character customisers and retail character shading

The user wants no customiser or retail shading in the app (see `game-code-out-plan.md`, Decisions). Delete:
- `customiser_parts.rs`, `customiser_material.rs` with `customiser_stamp.wgsl` and
  `customiser_prepass.wgsl`, and `custom_models.rs`;
- `retail_character.rs` with its wgsl, `retail_irradiance.rs` and `retail_shadow.rs`;
- `retail_exposure.rs` and `retail_tone.wgsl` (desktop only).

The Skate 3 skater then draws from its manifest GLB with the standard material, as it already does on the
headset. Character customisation comes back later through the engine contract, built into the headless
games.

**Headset check:** the Skate 3 skater looks unchanged, with no missing-resource panics on Skate 3's
first turn.

**Done (2026-11-16):**
- **Deleted:** all of the above, plus `retail_render.rs` (empty by then), `retail_exposure.wgsl`,
  `input::Navigation` (only the menus read it), the texture-codec fixtures, and
  `AnimationStatus::for_scene` with its two tests (it bound imported custom models).
- **Kept behaviour:** the profile's `preferences` system ran every frame. A `physics::preferences` system
  now applies the default profile the same way, in the same place (PreUpdate, before `poll_controllers`).
  Without a saved `settings/character.json` that is what the headset applied before: trucks and wheels
  0.7, stance 1, style and posture 0, stock gestures.
- The skater draws from its manifest GLB with `StandardMaterial`.

## Estimate

Passes 1-4: about 1-2 days. Pass 5 is larger.
