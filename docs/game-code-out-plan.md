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
   - (Done 2026-11-16: the decode path and the Start/Back/D-pad chords are removed.)

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

### Core and modes

The contract has a **core** that every engine implements, and a module per **mode** (`skate` now; `race`,
`platformer` and others later) that an engine of that mode also implements. Skating concepts live only in
the skate module.

**The sorting rule: when in doubt, it goes in the mode module.** Moving something from a mode into core
later is an addition, and no engine breaks. Moving something out of core breaks every engine. So core holds
only what a racing or platformer engine would need unchanged. This doesn't design the race contract (see
`buildar-proposal.md`: "don't build the abstraction ahead of the second mode"). It only keeps skate out of
core.

**Guard:** core must contain none of the words skater, board, deck, grind, rail, trick or park. That
becomes a test in `engine-core` once the crate exists.

### Where it lives

- **`engine-core`:** a new Bevy-free crate, with glam pinned to Bevy's version.
  - Core is the crate root. Each mode is a module: `engine_core::skate`.
  - `TouchState` moves into core. `engine_api::skate` moves into `engine_core::skate`.
  - `engine-api` stays as the app's Bevy wrapper, with newtypes and re-exports, split the same way.
- **Its own small repo:** both headless repos and the app pin it by revision. If it stayed in the app repo,
  the headless repos would depend on the app.

### Core

```rust
pub trait EngineCore: Send {
    fn info(&self) -> &EngineInfo;   // key, name, mode, controls sheet, data spec, settings schema
    fn period(&self) -> Duration;    // re-read after every step (Skate 3 slow-mo)
    fn set_input(&mut self, touch: &TouchState, held: PadHeld, dt: f32); // the engine maps Touch to its pad
    fn set_view(&mut self, eye_basis: Option<Mat3>);
    fn set_stage(&mut self, stage: &Stage) -> Applied;  // Applied::Deferred: the engine takes it later
    fn place_player(&mut self, point: Vec3, forward: Vec3);
    fn step(&mut self) -> Result<(), String>;
    fn focus(&self) -> Option<Focus>;                   // where the player is, for the app to follow
    fn models(&mut self) -> Option<Vec<ModelSource>>;   // once, after boot
    fn pose(&self, model: usize, out: &mut Pose) -> bool;
    fn hud(&mut self, wanted: bool, out: &mut HudFrame) -> bool; // false: unchanged
    fn texture(&mut self, id: u32) -> Option<Rgba8>;    // fetched once per id
    fn audio(&mut self, out: &mut AudioFrame);
    fn drain_events(&mut self, out: &mut Vec<Event>);
    fn environment(&self) -> Environment;
    fn settings(&self) -> Vec<Setting>;
    fn set_setting(&mut self, key: &str, v: f32);

    /// The skate mode's part, for an engine of that mode. Only the app's skate layer calls it.
    fn skate(&mut self) -> Option<&mut dyn skate::SkateMode> { None }
}
```

- **Booting:** `EngineFactory::boot(files)` runs on a worker.
- **One `step()` replaces the staged sim API.** Its hooks (session markers, multiplayer) are gone.
- **One space and unit** (`engine_core::stage`'s docs): the runtime's world frame (the room as the rig lays it out,
  y up) in metres as the game sees them (the player's world scale makes a real metre more or fewer). `Stage`,
  `Focus`, `Frame` and `PlacePlayer` are all in it; an engine converts inside itself (THUG2's inches, `OFFSET` and
  world size are `thug2_engine::stage`'s; Skate 3's and SK8's worlds are this space as it is).
- **`Stage`** is what every mode stands on (as built in step 5):
  - the room's flat faces as quads and its other faces as triangles (quads stay quads: Skate 3 treats them apart);
  - the placed objects' solid faces, each a `Solid { id, quads }` (a skate piece's body, then its rail tubes);
  - a revision number.
  `Applied { Done, Deferred }` says whether an engine took it; the runtime offers the latest every frame.
- **`PlacePlayer { point, forward }`**: put the player down; what "down" means is the engine's.
- **`Focus`** is a frame (feet, up, forward) and a drawn height. What's placed at the player, the sound emitters,
  the menus' distance checks, the room captures and carrying the player over to the next engine follow it. The app
  keeps one per engine (`engine_api::PlayerFocus`); a skate engine's comes from its `SkaterLink` (below).
- **Input:** `PadHeld { held, pause }` goes in with the `TouchState`; an engine's controls sheet comes back as
  `Control { input, action }` rows (the app adds its own after them).
- **`Pose`** has **body groups**: named sets of joints, each with its own frame (for example "rider" and
  "board"). Shadows cast per group, so separate bodies each cast their own.
- **`Event`** carries only neutral kinds (a sound request, a fault). Each mode adds its own kinds in its
  module.
- **The HUD placements** are neutral:

  ```rust
  Placement = Hand { metres_per_px, offset, anchor, hand: Option<HudHand>, on_left: Option<(Vec2, Vec2)> }
            | AtBody { group, offset, billboard, min_view_share }   // e.g. THUG2's balance meter at the rider
            | HeadCorner
  // as built: AtPlayer { side, width_per_height, least_view_share, middle, clear } at the Focus (body groups exist since step 6; AtBody not built yet)
  ```

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
```

### The skate mode (`engine_core::skate`)

```rust
pub trait SkateMode {
    fn set_park(&mut self, park: &SkatePark) -> Applied; // kept until set_stage brings its stage; taken together
    fn skater(&self) -> Option<SkaterLink>;
    fn trick(&self) -> Option<TrickText>;                // the generic readout when the engine has no HUD
}
SkatePark { revision, room_edges, pieces: Vec<ParkPiece { solid, rail_quads: Range<usize>, edges }>, rails }
SkaterLink { rider: Option<Frame>, board: Option<Frame>, on_board: bool, grinding: bool, size: f32 }
TrickText { name, points, line }
SkateEvent = Pop | Land | Bail | Grind | Trick { .. } | ...
```

- **`SkatePark`** is the skate annotations over the core stage, of the same revision: the room's grindable edges,
  each piece's grindable edges and which of its solid's quads are rail tubes (`rail_quads`, a range: Skate 3 gives
  them the convex rail flags, SK8 metal), and every rail (room edges first, then the pieces' rails, ledges and
  copings). A skate engine keeps the park (`set_park`) and applies it with the stage (`set_stage`), both or
  neither: **`Deferred` while grinding** (the grind refers to the rails it has), and the latest is taken after. The
  app no longer reads `grinding`.
- **Bowl holes left the contract:** the app cuts the room's level faces over a sunk bowl before building the stage
  (`real_world::open_over`), and only its own pointer reads the holes (`xr::link::SunkHoles`).
- **Placing the player** is core. "On the board" is what a skate engine does with `place_player` (its own lift
  over the point: Skate 3 5 cm, THUG2 10 cm, SK8 5 cm).
- **`Focus`** for a skate engine is the rider, falling back to the board (`SkaterLink::focus`, as tall as
  `skate::SKATER_HEIGHT` at its size). That rule is skate's mapping onto core.

### Rider and board are separate

Both engines can separate the skater from the board: off-board walking, bails, a board left behind.

- **Drawing is fine.** Each engine draws the board as bones in the skater's one skeleton (Skate 3's
  `board_bones` in its render pose; THUG2's skin palette), so a separated board is only bone positions.
- **What breaks today is `SkaterLink.deck`,** a single point. Skate 3 fills it from the board's physics
  body (`skate3_sim::Link::of`), so with the rider walking away the app's "skater" is the board.
- **The app's generic layer reads `SkaterLink.deck` today** and moves to core `Focus` and body groups:
  - the contact shadow (`xr/contact_shadow.rs`);
  - spawn aiming and placement (`xr/spawn.rs`);
  - the AR collision rebuild and plane captures (`xr/link.rs`, `xr/planes.rs`);
  - the menus' distance checks (`xr/ui/pages.rs`);
  - THUG2's sound emitter.

  Only the skate layer (the park builder's rail feeding, the trick readout) keeps reading the skate module.
  (Step 5: they read the stage and the running engine's `PlayerFocus` now. Step 6: the contact shadow reads the
  running engine's body groups, `engine_api::PlayerBodies`; only an engine with none, SK8, falls back to
  `SkaterLink.board`.)
- **Both cast contact shadows,** from body groups. Today only the board does: three fixed soft ellipses
  (deck and trucks, `xr/contact_shadow.rs` `casters`), and the rider casts none.
  - The board's group keeps that shape.
  - Every other group casts from its bone segments: each segment (parent to child) becomes a soft ellipse
    on the surface below, fainter and wider with height.
  - It stays analytic, with no shadow map (directional shadows cost the headset 13 fps).
  - Landed with step 6 (below). THUG2's rider casts none yet: its C API gives no parents or joint positions.
- **For an engine that draws its board as its own model,** `models()` returns a list, each with its own
  pose. That costs nothing for one-skeleton engines.

### Check: a headless Wipeout against core

Implementing a headless Wipeout uses each core method with no skating concept:
- `info` says mode `race`;
- `set_input` maps Touch to thrust, airbrakes and weapons;
- `set_view` is unused;
- `set_stage` takes the room and the track pieces' solid geometry;
- `place_player` puts the ship on the start grid or at a respawn;
- `step` and `period` run its fixed tick;
- `focus` is the ship, for its contact shadow and engine sound;
- `models` and `pose` give the ship, with one body group;
- `hud` draws speed, lap and position, by the hand;
- `audio` and `drain_events` give its sounds;
- `environment` and `settings` (speed class) as data.

What it needs beyond core (the track's spline, checkpoints, AI lines, race state) goes in a `race` module,
the same way skate's park annotations do. Nothing in core has to change.

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

**Step 1, as done (2026-11-16):** `engine-core` holds `Mode`, `TouchState`, `HudHand` and the HUD types
(`hud`: frames, draws, parts, placements, shading, blending), checked against both producers (THUG2's
`Thug2HudDraw`/`Thug2HudVertex` and its parts; Skate 3's `apt_scene::Draw`). A test keeps skate words out
of the core. `engine-api` re-exports them where the app always named them. Left for the steps that reshape
them: `ArWorld`, `SkaterLink`, `TeleportSkater` and `TrickHud` (step 5, as the stage and the skate module);
`PadHeld` (step 4, with input); the `EngineCore` trait itself (added piece by piece as its callers arrive,
complete by step 8).

**Step 2, as done (2026-11-16):** one HUD renderer in the app, `skate-game`'s `engine_hud.rs` and
`engine_hud.wgsl`. It takes a `HudFrame` and a texture fetcher. Each part gets its own camera and image, on a layer
from a pool (22-30), at the part's `resolution` (image pixels per canvas pixel). A `Hand` part becomes a `HandHud`
panel; an `AtPlayer` part is a billboard at the player's `Focus`. `Focus` and `HudPart::resolution` are new in
engine-core. Mesh and material slots are reused, and an unchanged frame is skipped. Everything it spawns is
`EngineOwned`. One material and shader cover both shadings and all three blends: `Modulate`
with `gamma` is THUG2's maths exactly, and `Add` is a pipeline key. THUG2's side (parts, sizes, views, placements, the
D3D draw → `HudDraw`) is the new Bevy-free crate `crates/thug2-engine` (`hud::frame`), which moves to thug2-headless
with `thug2-sys` in step 10. `thug2/hud.rs` and `hud.wgsl` are gone. The meter's skater frame comes from `thug2.rs` as
a `Focus` until step 5. Parity: 24 real THUG2 HUD frames (grind and manual meters, the score, trick strings) and a
synthetic every-blend frame were recorded and drawn through the old path. Replayed through the new one, all 75 part
pictures are identical (max difference 0). A GPU test checks each shading and blend against its maths (within
1/255).

**Step 3, as done (2026-11-16):** Skate 3's HUD is on the same renderer. Its side moved to a Bevy-free crate in the
app's workspace, marked Skate-3-only so it can move into skate3-headless as it is: `crates/skate3-engine`
(`hud::Hud`). It covers loading `private/hud`, the `hud_runtime` advance, `apt_scene::draw` → `HudFrame` (one part:
`AREA` of the 1280x720 stage at `resolution` 1.5), the panel constants and texture name → id. Textures are handed
over once each, not kept twice. A frame is drawn only after an advance. skate3-headless itself is unchanged. The
app's adapter is `skate3_hud.rs`. It loads on the `AsyncComputeTaskPool`, advances per tick, draws per frame, and
keeps the multiplier-sound choice per tick (it reads `SkaterRuntime` and `skate_audio`, both still app-side). The
panel appears only once the HUD has loaded, so without `private/hud` the corner readout stays. `scoring_hud.rs` and
`hud_render.wgsl` are gone, and layer 31 is back in the pool (22-31). Parity: the real HUD (this machine's
`private/hud`) through a line's ten frames, old path and new at the headset's 960x540, is identical (max difference
0). The first try pre-scaled the stage by 1.5 instead of `resolution`; that was 2/255 off in 21-38 pixels of three
frames (glyph sampling).

**Left over from steps 2-3:** `engine_hud.rs`'s `at_player` still hard-codes the geometry factors of
THUG2's meter pictures (0.55, 0.3, 0.15, 0.2). They belong to the engine: move them into
`Placement::AtPlayer` (e.g. a picture anchor and a lift) when the next engine places a part at the player,
or with step 5. (Done in step 5: `middle` and `clear`.) The Skate 3 multiplier-sound choice is still in the app's
Skate 3 adapter (it reads app-side `SkaterRuntime`/`skate_audio`); it moves with step 7.

**Step 4, as done (2026-11-16):** each engine maps the Touch controllers to its own pad, in its crate.
- **Core:** `PadHeld { held, pause }` (engine-api's `PadHeld` resource wraps it, `Deref`), and `Control { input,
  action }`, an engine's controls-sheet row.
- **Skate 3** (`skate3_engine::input`): `touch_to_pad`, `CONTROLS`, and `ExternalPad`, the slot-0 packet the sim polls,
  with `set_input(&TouchState, PadHeld)` (neutral while `held`; the menu button keeps the left stick and X).
  `ExternalPad` and `poll_controllers` are the Skate 3 adapter's (`input.rs`), no longer the XR layer's: the resource
  is always there (disconnected until a state is set, as before on the desktop), and `pad_from_touch` moved out of
  `xr/link_retail.rs`.
- **THUG2** (`thug2_engine::input`): `pad`, `CONTROLS`, and `Input { pad, hold }` with `set_input(&TouchState,
  PadHeld)` (at rest while `held` or the menu button is down).
- **The app** keeps `TouchState`, the Bluetooth fold-in (`xr::touch::pad_into_touch`), the held flags, placement, and
  its own sheet rows (`HOST_CONTROLS`: aiming the skater, the menu, build mode), listed after an engine's. SK8
  (app-resident, `clean/input.rs`) keeps its whole sheet, the shared rows in it.
- No `EngineCore` trait yet; the functions have the shapes its methods will take (`dt` unused so far).

**Step 5, as done (2026-11-16):** `ArWorld`, `SkaterLink`, `TeleportSkater` and `TrickHud` are split by the
sorting rule.
- **Core** (`engine_core::stage`): `Stage { revision, room_quads, room_triangles, solids }`, `Solid { id, quads }`,
  `Applied`, `Frame`, `PlacePlayer`, the one space and unit; `Focus` gains `forward`;
  `Placement::AtPlayer` gains `middle` and `clear` (THUG2's meter factors, filled by `thug2_engine::hud`).
- **Skate** (`engine_core::skate`): `SkatePark`/`ParkPiece`, `SkaterLink { rider, board, on_board, grinding, size }`
  with `focus()`, `TrickText`, `SKATER_HEIGHT`.
- **engine-api** wraps them: `CurrentStage`, `PlacePlayer` (a message), `PlayerFocus` (the one focus map: it was
  `engine_hud`'s `HudFocus`), `skate::{CurrentPark, SkaterLink, TrickHud}`.
- **The app** (`real_world::assemble`) builds the stage and park together and always offers the latest; it rebuilds
  mid-grind now. Generic consumers read the stage (spawn aiming, build mode, room light, controller shapes, contact
  shadow patches) or the running engine's focus (planes' free-roam floor, the world-scale re-place, carrying the
  player over, `THUG2_SHOT`); a skate-layer system (`xr::link::focus_from_link`) makes the focus from the link. The
  contact shadow read `SkaterLink.board` until step 6.
- **Skate 3** (`skate3_engine::stage`, `::trick`): `StageFeed` (`set_park`, `set_stage` with the grind deferral and
  the sim's `set_real_surfaces`/`set_grind_rails` arguments rebuilt from stage and park), `place_player` (the travel
  matrix, 5 cm up), `link` (board: the deck body; rider: the animation root `animation_to_world`, its stance axes
  turned as the sim's effective root does: built from the board while riding, reset from the biped's own world
  frame off it, as read in the sim; not yet checked on the headset; on the board unless in the biped states), `trick::text` and
  `readable_trick`.
- **THUG2** (`thug2_engine::stage`): units (`to_game`/`from_game`, `OFFSET`), face classes (`WALL_UP`/`RAMP_UP`/
  `VERT_UP`, `triangles`, `piece_faces`), `DECK_HEIGHT`, `SOUND_HEIGHT`, `spawn`, `link`, and `StageFeed` (the grind
  deferral, a new world size re-placing the skater, waiting for a place to put it). C API gap: one skater frame, no
  board of its own and no off-board state, so its board is the rider's frame `DECK_HEIGHT` up and `on_board` is
  always true.
- **SK8** (app-resident) takes the stage and park in `clean::sim` (its own grind deferral, as before); its rider and
  board are one frame (the engine gives the rider's state only).
- **Parity:** before the split, `xr::stage_parity` recorded the `f32` bits of what each engine was given (Skate 3's
  four `set_real_surfaces` arguments and `set_grind_rails`, THUG2's classified faces and rails at world sizes 1 and
  2.5, SK8's collision triangles with surfaces and its rails) for three worlds: the room alone, the room with a piece
  of every kind, and both through a turned, scaled rig. Through the stage and park they are identical. The meter's
  placement is checked against the old factors exactly (`engine_hud` tests).

**Step 6, as done (2026-11-16):** each engine describes its skater model and gives its pose as plain data; the app
draws both through one generic layer that doesn't know which engine it is.
- **Core** (`engine_core::model`): `Model { source, parents, groups }`; `ModelSource::Gltf { path, joint_names, basis }`
  or `ModelSource::Meshes { meshes, textures, joint_count }` (`MeshData`: positions, normals, uvs, colour multipliers,
  joints and weights, indices, a `MaterialDesc { base_color, texture, alpha, two_sided, vertex_color, lighting }` with
  `AlphaMode` and `Lighting::{Standard, Unlit}`; textures are the HUD's `Rgba8`); `Pose { generation, discontinuity,
  root, joints }`. The skeleton's shape is the model's (`parents`), not each pose's: the one refinement of the sketch
  above. Body groups (`BodyGroup`, `Cast`, `Point`, `Ellipse`) are in core with neutral `u32` ids;
  `engine_core::skate::body` names them (`RIDER`, `BOARD`) and holds `BOARD_FOOTPRINT`.
- **The app** (`skate-game`'s `model.rs`): `PoseHistory` (the last two poses, taken once; a cut, a new step period or
  joint count restarts the pair) with `blend`; for a glTF source `bind_skins` (skin joints by name, the nearest named
  ancestor as parent), `joint_locals` (bone-local blending) and `joint_transforms` (unblended); for meshes as data
  `spawn_meshes`, `mesh`, `material` (the lit or unlit standard material, roughness 0.85) and `flat_joints` (root x
  joint, identity inverse binds, not blended). The per-engine systems that feed it stay in each adapter until step 8.
- **Skate 3** (`skate3_engine::model`): `model` (the manifest's GLB, the rig's bone names and parents, `BASIS`, the old
  `render_basis` to the sign of its zeros) and `pose` (the render pose and animation root; a discontinuity when the
  gameplay camera cuts or the teleport flag is up: the sim's own flag, so skate3-headless is unchanged). The desktop
  camera's shot rides in the same `PoseHistory` as the pose, so camera, root and joints keep one interval; the replay
  (parked) samples the same snapshots.
- **THUG2** (`thug2_engine::model`): `source` (winding, vertex colours /128, the part colour doubled, weights of three
  normalised, alpha mode) and `pose` (the w columns fixed, game to ours x draw matrix as the root, the palette as the
  joints). Its only interpolation, the root moved back by the skater's position between the game's last two frames,
  stays exactly as it was: the adapter works out the shift (game units) and the engine puts it between the unit
  conversion and the draw matrix. A generic world-space root blend instead would also turn the root between frames,
  and would take the motion from the draw matrix rather than the skater's position: the user's call.
- **Parity:** `model_parity` (recorded at 8773c47, the old maths lifted out as functions unchanged) digests the `f32`
  bits of what each engine's model is given. Skate 3: joints and root over fixed ticks (a cut, a cadence change, a
  repeated generation, a bone count change) at six alphas, inside and outside 0..1, and the no-pair fallback; and
  the real game (`SKATE3_ASSET_ROOT`): 150 of its own ticks bound to its GLB. THUG2: joints from a fixed palette and
  draw matrix (short palette, junk w columns, world sizes 1 and 2.5, moving and not) and its parts as meshes and
  materials; and the real game (ignored: the game boots once a process) after a fixed run. All identical through the
  new path.
- **Contact shadows from body groups** (`xr/contact_shadow.rs`): the running engine's bodies, placed by the model
  layer each frame (`model::bodies` into `engine_api::PlayerBodies`; THUG2's after its pose, ordered by
  `engine_api::BodiesSet`). A footprint body casts its ellipses (the board: `BOARD_FOOTPRINT`, today's deck and two
  trucks, sizes in focus heights); every other body casts one soft ellipse a bone (parent to child, both in the
  group), round its line seen from above, under its middle, fainter than the board (0.35 against 0.65) and fainter and
  wider with height as before. Bones under 6 cm (at size 1) cast nothing; the longest 20 are kept. The shader's array
  went from 4 casters to 24 (`MAX_CASTERS`): the material's uniform from 272 to 1,552 bytes, and each shadow pixel loops
  over the casters there are (one past its ellipse skips the `exp`). The patch is cut round all the bodies (at least a
  size, at most three sizes out; bodies farther apart than that, a board left far behind, keep it round the player's
  focus, so the rider keeps its shadow). With no bodies (SK8, and before a pose) the board's footprint is cast from
  `SkaterLink.board` as before: that picture is identical to the old one (`a_picture_of_contact_shadows`, max difference
  0; `SK8_SHADOW_RIDER=1` adds a standing rider's bones).
  - **Skate 3** (`skate3_engine::model`): the board is the rig's `SKATEBOARD_ROOT` with its trucks and wheels (the
    sim's `board_bones` and their parent), cast from the back truck to the front; the rider is the hips' tree (23
    bones; the trajectory root and the hands and toes reparented to the board for its IK are neither). Checked on the
    real game: the footprint's middle within 10 cm of the sim's deck body and along it.
  - **THUG2** (`thug2_engine::model::board`): the board is the parts lying under 8 inches and at least 20 long in the
    bind pose (the deck, trucks and wheels: five parts), its joints those only they are skinned to (four of the
    palette's), its ends points of the bind pose carried by the joint weighing most at each. The rider casts nothing:
    the C API gives no parents and no joint positions (the palette is inverse bind times posed), and guessing a
    skeleton from shared skin weights is a workaround this step doesn't take. Checked on the real game: the
    footprint's middle within 15 cm of the board in the link and along it.

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

## Noted for later (headset check, 2026-11-16)

- **Spawn facing:** the spawn aiming circle gets a triangle off its edge showing which way the player will face, turned
  with a twist of the wrist.
- **Controls page:** revisit it; as it stands it is hard to use.
- **Park settings page:** its own page in the Parks page's design language, replacing the legacy one. The size slider
  is poor: the world jumps about while it is dragged (scale on release, or a preview, instead).

## Known bugs (2026-11-16)

- **World scale moves the player.** Changing the skater size slider still leaves the player off their real spot in
  every engine, even with the single placement after the slider settles (`xr::link::keep_spot_on_rescale`). Left
  for the park settings page rework. Reported as new since steps 4-5: the first suspect is step 5's (9701b9d) swap
  of the deck for `Focus::feet` as the point re-placed (an engine's placement puts its deck or spawn point there, not
  its rider's feet).
- **THUG2: hitting a wall fast enters it.** At speed the skater can go into a wall and skate along its plane; seen
  on the build with the wall slab (`thug2_engine::stage::WALL_SKIN`, cb8a75d). On a flat wall in the real engine
  (`thug2-engine/tests/wallplant.rs`, 3fd5397) it predates the slab and the slab fixes it: straight-on jumps planted
  by accident (the ollie's press counts as the plant's) and hit the plant-placement bug; rolling and glancing hits
  never went in. Still seen with the slab, so the next suspects are what that test doesn't cover: room-mesh
  triangles and placed pieces' sides.

## Later: several models from one engine (2026-11-17)

An engine may hand over several models at once, such as the CPU racers in a headless Wipeout or MKWii. Today every
engine has one model (the skater). The pose history is keyed by a model id already (generic pose interpolation, after
step 6); the rest waits for an engine that needs it.

- **Contract:**
  - `models()` returns a list, each model with its own poses, matched by model id.
  - A model's look is separate from its instances: eight karts with one look load it once and are posed eight times.
  - Models come and go (a CPU spawns or drops out), and the engine says so.
  - One model is the player's: it gives the core `Focus`. The others are only drawn.
- **App:**
  - Spawn and despawn models as the engine says, with one pose history per model.
  - **A shadow budget.** The caster cap (24) is sized for one skater. Nearby models get their body-group shadows,
    distant ones a single soft blob or none. Measure the frame rate on the headset with a full grid.
  - One-model engines (Skate 3, THUG2, SK8) don't change.

## Later: a Mario Kart Wii spike (2026-11-17)

[Wiicompiled](https://github.com/patchzyy/Wiicompiled) is a static recompilation of Mario Kart Wii (GPL-3.0, from the
player's own disc). The game's PowerPC code is lifted to C++ and built with a runtime that draws through aurora (GX
over Dawn) and reads input with SDL. Unlike thug2-decomp it gives behaviour, not readable code: hooks would be per
lifted function, named from the MKW community's symbol maps.

- **The spike (Linux x64):** build it, step the game with aurora stubbed out, and load a custom KCL. If the runtime
  can't run without its renderer, the cost rises a lot.
- **The lifts if it passes:**
  - A headless build for Android ARM64 (its tested build is x86-64 desktop), with no aurora, SDL or Dawn.
  - The stage as a course, made at runtime: KCL (collision, mechanical) and KMP (start, checkpoints, respawns, CPU
    and item routes: the `race` mode module's design work). Free roam or time trial first sidesteps most of KMP.
  - Karts and riders in the shared renderer: models from the game's files and bone matrices from game state (as
    THUG2's palette), or aurora's draw calls captured. GX's colour-combining (TEV) materials map lossily onto
    `MaterialDesc`; the HUD needs its layouts as draw lists.
- Several models from one engine (the CPU racers) is the section above.
