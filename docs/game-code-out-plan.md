# Game code out of the app: the engine contract

Status: study, 2026-11-16, checked against app `198b719`, skate3-headless `ff026ef` and thug2-headless
`13b2201`. It follows from the buildAR direction: the app is a generic runtime, game code lives in each
engine's headless repo, and the app hosts the shared renderer. Prerequisite: the passes in
`legacy-refactor-plan.md`.

## Findings that set the scope

1. **The retail renderer does nothing on the headset.**
   - `retail_character::load` returns unless there is a map (`retail_character.rs:146-151`), so the Skate 3
     skater draws with its GLB's `StandardMaterial`, the same as THUG2's.
   - Tone and exposure sit only on the desktop `GameplayCamera`.
   - World, sky, backdrop and shadows are map-only (legacy pass 5).
   - So a renderer-neutral material description loses nothing the Quest shows today.
2. **Both HUDs are already the same data:** triangle lists of 2D vertices, one texture each, and a colour
   transform.
   - Skate 3: `apt_scene::Draw`.
   - THUG2: `Thug2HudDraw`/`Thug2HudVertex`, plus a blend mode and colour op.
   - The two shaders are 9 and 25 lines. This makes the HUD the pilot.
3. **Some Skate 3 audio and input code is dead on the headset.**
   - Stream, bank and ambience decoding (`skate_audio.rs:253-815`) is only reached through
     `SKATE_AUDIO_PLAY*` environment variables.
   - Most of `TouchMapper`'s chords have no effect (Start is masked; replay is parked).

## Inventory

Classes:
- **(a)** a generic host capability that is only named after an engine: generalize it and keep it.
- **(b)** game knowledge: move it to the engine's repo.
- **(c)** a mix: cut it as described.

### Skate 3

| Code | Class | Cut |
|---|---|---|
| `scoring_hud.rs` | c | **(b):** `private/hud` loading, `hud_runtime` advance, `apt_scene::draw`, the multiplier-sound choice, the crop and panel constants. **(a):** draw list → mesh/material, render target, `HandHud` quad. |
| `xr/link_retail.rs` | c, mostly b | **(b):** pad mapping, `apply_world` (grind deferral), teleport, pose, `readable_trick`. **(a):** `hold`, the eye basis. |
| `retail_render`/`sky`/`backdrop`/`shadow_*` + wgsl | b, dead | Removed with legacy pass 5. |
| `retail_exposure`, `retail_tone` | b, desktop-only | Delete. |
| `retail_character.rs` + wgsl | c, inactive | Retail lighting data is (b). The SH/key/rim lighting model could become a generic "SH character" material. |
| `customiser_parts`/`material`/`custom_models` | b | The retail CAC formats. Decision needed (below). |
| `animation.rs`, `presentation.rs` | c | Joint binding by name and pose blending are (a). The bone source, `render_basis` and the reset flag are (b): the sim should report a discontinuity itself. |
| `camera.rs` | b | Inactive in XR; becomes internal to the sim. |
| `physics.rs`, `graph_runtime`, `grind_world`, `assets.rs` | a (shims) | Collapse into the generic driver. |
| `input.rs`, `xr/touch.rs` | c | `TouchMapper` (Touch → Xbox pad) is (b). Folding a Bluetooth pad into Touch is (a). |
| `world.rs` | c | Spawning a model is (a). Clear colour, ambient 350 and key light 11,000 lux are (b) data, which the room light fades. |
| `skate_audio.rs` | c | `LivePcm` is (a). The decode path is (b) and dead. `FrontEndSoundRequest` becomes a sim event. |
| `player_audio.rs` | a | The render thread. |
| `player_audio_clean.rs` | c | The CC0 driver is (a), shared with SK8. Reading Skate 3's observation is (b): it becomes neutral events. |
| `stream_pcm`, `binaural`, `reverb`, `clean_mix`, `clean_synth` | a | Keep. |
| `setup.rs`, `clean_pack.rs` | b | Becomes `Bundle::open` validation. |
| `app.rs` | c | The host builder and `Hold` are (a). `add_retail` wiring, the Android asset-source hack and the map origin are (b). |
| `xr/ui/import.rs`, the "Skate 3 data" page | c | Scan, copy and unzip are (a). Folder names and markers are (b) data. |

### THUG2

| Code | Class | Cut |
|---|---|---|
| `thug2.rs` (1,265 lines) | c | **(a):** boot on a worker, crash marker, accumulator, voices → emitters, skinned-mesh spawn and pose apply, stand-in. **(b):** controls and `pad()`, units and `OFFSET`, face classification (wall/ramp/vert), deck and sound heights, material fixups (vertex colour /128, ×2, winding, palette w column), file layout. Its tuning becomes settings expressed as data. |
| `thug2/hud.rs` (663 lines) | c | **(b):** `Part::of`/`view` (which draw is which part), sizes, layers. **(a):** the paint and glow materials, draw → mesh, the meter billboard at the skater. |
| `crates/thug2-sys` | b | Moves into the thug2-headless repo. |

### SK8

`skate-lite-sys` exports rider state only. The app supplies the look, input, menus and sounds in `clean/`
(5.4k lines). By the rule this is game code too, but it is the store build's only engine, so it goes last.
`clean::wanted`, the font embedding and `add_xr_engines` are host jobs to move out of `clean/`.

## The contract

### Where it lives

- **`engine-core`:** a new Bevy-free crate, with glam pinned to Bevy's version.
  - It holds plain data and the `EngineCore` trait.
  - `engine_api::skate` and `TouchState` move into it.
  - `engine-api` stays as the app's Bevy wrapper, with newtypes and re-exports.
- **Its own small repo:** both headless repos and the app pin it by revision. If it stayed in the app repo,
  the headless repos would depend on the app.

### The trait

```rust
pub trait EngineCore: Send {
    fn info(&self) -> &EngineInfo;  // key, name, mode, controls sheet, data spec, settings schema
    fn period(&self) -> Duration;   // re-read after every step (Skate 3 slow-mo)
    fn set_input(&mut self, touch: &TouchState, held: PadHeld, dt: f32); // the engine maps Touch to its pad
    fn set_view(&mut self, eye_basis: Option<Mat3>);
    fn set_world(&mut self, w: &ArWorld) -> Applied;                     // Deferred while grinding
    fn teleport(&mut self, point: Vec3, forward: Vec3);
    fn step(&mut self) -> Result<(), String>;
    fn link(&self) -> Option<SkaterLink>;                                // metres
    fn model(&mut self) -> Option<ModelSource>;                          // once, after boot
    fn pose(&self, out: &mut Pose) -> bool;
    fn hud(&mut self, wanted: bool, out: &mut HudFrame) -> bool;         // false: unchanged
    fn texture(&mut self, id: u32) -> Option<Rgba8>;                     // fetched once per id
    fn audio(&mut self, out: &mut AudioFrame);
    fn drain_events(&mut self, out: &mut Vec<GameEvent>);
    fn environment(&self) -> Environment;
    fn settings(&self) -> Vec<Setting>;
    fn set_setting(&mut self, key: &str, v: f32);
}
```

- **Booting:** `EngineFactory::boot(files)` runs on a worker.
- **One `step()` replaces the staged sim API.** Its hooks (session markers, multiplayer) are gone.

### HUD

The engine decides the layout. The app draws it and places it.

```rust
HudFrame {
    canvas: [f32; 2],
    vertices: Vec<{ pos: [f32; 2], uv: [f32; 2], color: [u8; 4] }>,
    draws: Vec<{ first, count, texture: Option<u32>, part: u8, blend: Alpha|Add|Opaque,
                 mul: [f32; 4], add: [f32; 4], vertex_scale: f32, gamma_space: bool }>,
    parts: Vec<{ name, size_px, crop: Rect, empty, placement: Placement }>,
}
Placement = Hand { metres_per_px, offset, anchor, hand: Option<HudHand>, on_left: Option<(Vec2, Vec2)> }
          | AtSkater { arc_over_head | beside, per_skater_height, min_view_share }
          | HeadCorner
```

### Rider and board are separate

Both engines can separate the skater from the board: off-board walking, bails, a board left behind.

- **Drawing is fine.** Each engine draws the board as bones in the skater's one skeleton (Skate 3's
  `board_bones` in its render pose; THUG2's skin palette), so a separated board is only bone positions.
- **What breaks is `SkaterLink`.** Today it has one point, `deck`. Skate 3 fills it from the board's
  physics body (`skate3_sim::Link::of`), so with the rider walking away the app's "skater" is the board.
  Everything that follows the skater follows that one point:
  - the contact shadow (`xr/contact_shadow.rs`);
  - the AR collision rebuild and plane captures (`xr/link.rs`, `xr/planes.rs`);
  - THUG2's sound emitter;
  - the menus' distance checks (`xr/ui/pages.rs`).
- **The contract reports both:**

  ```rust
  SkaterLink { rider: Option<Frame>, board: Option<Frame>, on_board: bool, grinding: bool, size: f32 }
  ```

  Followers take the rider (falling back to the board). The contact shadow draws one per body that is
  present. Teleport still puts the rider back on the board.
- **For an engine that draws its board as its own model,** `ModelSource` takes a list of models, each with
  its own pose. That costs nothing for one-skeleton engines.

### Model and pose

- **Renderer-neutral, not shaders-as-data.**
  - Skate 3: `ModelSource::Gltf { path, joint_names, basis }`.
  - THUG2: `ModelSource::Meshes { meshes, textures }`.
  - A `MaterialDesc` describes base colour, texture, alpha mode, two-sided, vertex colour, and lit or unlit.
- **The app owns a few lighting models:** Standard, Unlit, and later perhaps "SH character".
- **Pose:** `Pose { generation, discontinuity, root, joints, parents }`.
- **Shaders as data only if retail character shading is revived.** The WGSL would then ship from
  skate3-headless as text that the app compiles and tests.

### Audio, input, environment, camera

- **Audio:**
  - THUG2: `AudioFrame::Voices` (new sounds and voice state).
  - Skate 3: a pull stream via a *factory* run on the app's audio thread, because `PlayerAudio` isn't
    `Send`.
  - Gameplay `GameEvent`s drive the CC0 fallback and the multiplier sound, per tick.
- **Input:**
  - The engines own their mapping: `TouchMapper`, `pad()`, `clean/input.rs`.
  - The app owns `TouchState`, the Bluetooth fold-in, `PadHeld` and placement.
  - The controls sheet comes back as data.
- **Environment:** `Environment { clear, ambient_lux, key: (dir, lux, shadows), sh }`. The app applies it
  and the room light fades it.
- **Camera:** the eye basis goes in; nothing comes out in XR. The contract is in metres.

### Boundaries

- **THUG2:** only plain `repr(C)` data crosses the C API. Its game knowledge moves to a Rust crate in
  thug2-headless (with `thug2-sys`) that implements `EngineCore`.
- **Skate 3:** skate3-sim implements `EngineCore` directly.

## What the app keeps

- One draw-list HUD renderer, with per-part targets, a render-layer pool, hand placement, the at-skater
  billboard and the corner fallback.
- A generic skinned-model renderer: GLB joint binding, raw meshes, pose blending, `MaterialDesc`
  materials and the stand-in.
- Audio: voice emitters, the pull-stream thread, `LivePcm`/`StreamPcm`, the CC0 driver on events,
  binaural, reverb.
- A generic engine driver:
  - worker boot and the crash marker;
  - the accumulator driven by `period()`;
  - the `Hold` pause semantics;
  - settings pages and `CourseSettings`.
- The registry, the XR layer, courses and menus.
- One data installer driven by `EngineInfo`.
- One host builder.

## Migration

| # | Step | Size |
|---|---|---|
| 1 | `engine-core` repo: HUD types, `TouchState`, the skate data types; `engine-api` re-exports them | S |
| 2 | The generic draw-list HUD renderer, **THUG2 first**: parts, crops and placements come from the THUG2 side; `thug2/hud.rs` and `hud.wgsl` go. Diff `SK8_THUG2_HUD_SHOT` pictures before and after. | M |
| 3 | The Skate 3 HUD into the sim (`skate3_sim::hud`): load, advance, draw, layout, texture ids, the multiplier sound as a tick event. It still loads off the main thread. | S-M |
| 4 | Input mapping into the engines; `ExternalPad`/`poll_controllers` leave the XR layer | S |
| 5 | World, teleport, link and trick text into the engines (grind deferral; THUG2 face classes and units) | S-M |
| 6 | Model and pose: generic binding and blending; engines provide the source and pose | M |
| 7 | Audio: voices, pull stream and events; the dead decode path removed | M |
| 8 | One generic driver replaces `thug2.rs`, `link_retail.rs` and `add_retail`; one host builder; Skate 3 boots lazily; the Android asset-source hack goes | L |
| 9 | A generic data installer and pages | S |
| 10 | `thug2-sys` moves into thug2-headless | S |
| 11 | Optional: SK8 onto the contract | M |

### Risks

- **Quest performance:**
  - Copies across the boundary are small (a palette is about 4 KB a frame; meshes and textures are copied
    once).
  - The real cost today is `scoring_hud::render` building new mesh and material assets for every draw,
    every frame. The generic renderer should batch per part, skip unchanged frames, and use `u32` texture
    ids.
- **Gamma:** THUG2 blends in gamma space. Keep the flag and check with picture diffs.
- **Skate 3's variable period:** the driver must re-read it after every step.
- **`PlayerAudio` isn't `Send`:** build it on the audio thread.

## Decisions (2026-11-16)

1. **Character customisers are stripped from the app.** The customiser parts, materials and custom models
   go, along with the retail character code that uses them. The Skate 3 skater is its manifest GLB.
   **To return to:** the user wants Skate 3's and THUG2's own character customisers later. They belong at
   the headless-game level and would be exposed through the engine contract (for example, an engine's
   customisation options and the resulting model/material description), not built into the app.
   **Done (2026-11-16, legacy pass 6):** the customiser parts and material, custom models, retail character
   shading, irradiance, shadow state, and exposure/tone are deleted. The skater's gameplay preferences are
   always the default profile's.
2. **No retail character shading, and no shaders-as-data.** The shared renderer keeps the engines
   cohesive with the stages. Engines describe models and materials as plain data, and the app's lighting
   models draw them.
3. **`engine-core` lives in its own repo**, pinned by revision like skate3-headless.
4. **THUG2's game knowledge is Rust,** in a crate in thug2-headless that also takes in `thug2-sys`.
