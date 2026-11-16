# buildAR: an AR course-building runtime with modes

Status: proposal, 2026-11-16. Step 1 is under way. It sits next to `engine-packaging-proposal.md` and
`skate3-sim-split.md`.

## The idea

Generalize sk8ar into **buildAR**, a runtime for building courses in your real room and playing them with
whichever engine fits.

- **Engines implement modes.** Skate 3, THUG2 and SK8 implement **skate**. Race engines (a headless
  Wipeout, possibly Mario Kart Wii) would implement **race**.
- **Each mode has its own course type and its own tools.**
  - Skate builds **parks** with the piece builder.
  - Race builds **tracks** with a spline editor.
- **The shared runtime is everything else:**
  - the room and its collision, anchors and co-location;
  - world scale;
  - the engine registry and switching;
  - HUD panels by the controllers;
  - the menus, and saving and placing courses.

## What already generalizes

Most of the app is already mode-neutral:

- **The XR layer:** room scan as collision, spatial anchors, world scale, occlusion, room light and
  contact shadows.
- **Engine management:** the registry with declared ownership (`EngineSet`, `EngineOwned`), switching, and
  startup on first enter.
- **Input:** controller input published once (`LatestTouch`), with the shared layer's claim on it
  (`PadHeld`: `held` for a neutral pad, `pause` for pausing).
- **HUDs:** panels drawn by an engine and placed by a controller (`HandHud`, `EngineHudCamera`).
- **Courses:** saved with anchors, pose and scale, placed and scaled in the room, chosen from the menus. The
  co-location plan (shared frames, opt-in room mesh) is mode-neutral too.

## What is skate-specific today

- **In the engine API:**
  - `SkaterLink` (a deck pose and whether it grinds);
  - the pieces, grind rails and edges, and bowl holes in `ArWorld`;
  - `TrickHud`;
  - `TeleportSkater`.
- **Park files:** pieces in the park frame. The engine tag and the engine's settings are mode-neutral.
- **The builder:** piece placement. This stays as skate mode's tool.

## What race mode would add

- **Course data:** a track surface with materials (road, offroad, boost, wall), checkpoints, respawn
  points, a start grid, AI routes, and race state (laps, positions).
- **A spline editor:** a centreline with width, banking and surface. It reuses placing, scaling and saving
  from the builder.
- **A spline-to-track compiler for each engine.** This is the real cost (see below).

## Where it is harder than "abstractions over sk8ar"

1. **Race engines need generated data, not just collision.**
   - The skate engines ride arbitrary triangles, which is why a scanned room works as a course. Racing
     games mostly don't.
   - Wipeout's ships are bound to its track sections, a chain of segments. We would generate its track
     format from the spline.
   - Mario Kart Wii needs its collision format with surface types, plus checkpoints, respawns, AI paths
     and the start.
   - In race mode the room is mostly scenery, or walls at most.
2. **Scale changes the experience.** A race track in a living room is tabletop or floor-level RC scale,
   roughly 1/20 to 1/50. The rig already scales. Readability at that size and speed, the camera, and a HUD
   seen from a distance need design, not just abstraction.
3. **The engines aren't equally ready** (from what I know; worth checking before committing).
   - **Wipeout:** phoboslab's `wipeout-rewrite` is a complete, portable C rewrite of the PSX game. It is
     close to ideal for a headless library shaped like thug2-headless.
   - **Mario Kart Wii:** the decomp is a matching Wii PowerPC decomp, not a PC port. Running it headless
     means replacing GX, the Wii's I/O and more, which is far bigger than THUG2, which was already a PC
     game. Treat it as speculative.
4. **Legal risk differs by rights holder.** Nintendo enforces much harder than THUG2's or Wipeout's rights
   holders. Even with "you supply the game", a public Mario Kart engine carries real takedown risk.
5. **Don't build the abstraction ahead of the second mode.** Designing race mode's contract with only skate
   behind it means guessing. Split what is clearly core now, and let a real race engine force out the race
   contract.

## Plan

1. **Now (step 1): split the engine API into a core contract and a skate extension.**
   - Rename `skate-engine-api` to `engine-api`. The root of the crate holds the core contract. A
     `skate` module holds `ArWorld`, `PieceSurfaces`, `SkaterLink`, `TeleportSkater` and `TrickHud`.
   - Add `Mode` (`Skate` for now). `Engine::mode()` says which mode an engine implements.
   - Add a `mode` tag to park files. A file without one is a skate park. A file of a mode this app
     doesn't know is left out of the list, so a course from a newer build isn't opened as the wrong thing.
   - Leave `ArWorld` whole for now: its room half (room quads and triangles) belongs in core, but moving it
     is for when race mode needs it.
   - Leave the app's name alone until a second mode exists.
2. **The first race engine: Wipeout.**
   - A headless wipeout-rewrite with the shape of thug2-headless: the sim and the app's own rendering,
     with no game files shipped.
   - A minimal spline editor and a spline-to-track compiler.
   - This proves whether race mode is fun in AR before more is generalized.
3. **Then generalize what that proved:**
   - the race contract and the track course type;
   - `ArWorld`'s room half moved to core;
   - per-mode builders behind a mode switch in the menu;
   - rename the app to buildAR.
4. **Mario Kart Wii only if its decomp becomes portable,** and with the legal question settled.

## Open questions

- Does a course keep one mode for life, or can a room hold a park and a track at once? Proposed: one mode
  per course; a room can hold several courses.
- Does the builder's piece kit become mode-specific (ramps vs. track pieces), or do some pieces (walls,
  boxes) belong to both?
- Race at room scale, or always a tabletop? This affects anchors, occlusion and how much the room matters.
