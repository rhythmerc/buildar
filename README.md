# buildAR for Meta Quest 3 (alpha)

Build a course in your real room, then play it with a real game's engine. buildAR is a passthrough AR runtime
for Quest 3. It scans your room and lets you build on it with the Touch controllers. A game engine then plays on
your floor, your furniture and what you built, drawn and heard as if it were in the room.

Today it has one mode, **skating**, and two engines:

- **Skate 3**: the faithful Rust port of Skate 3's skating
  ([skate3-headless](https://github.com/rhythmerc/skate3-headless)). It has the retail skater, animations, controls,
  trick display and player sounds.
- **THUG2**: Tony Hawk's Underground 2's own code, from
  [thug2-decomp](https://github.com/CanuckAussieKev/thug2-decomp), run headless
  ([thug2-headless](https://github.com/rhythmerc/thug2-headless)).

Ride your floor, tables and couch, and build a park of ramps, rails, ledges, quarter pipes and stairs. Switch
engines at any time: each carries on where it left off.

**[Download the latest APK](https://github.com/rhythmerc/buildar/releases/latest)** and install it with
`adb install -r buildar.apk` (or SideQuest). Open **buildAR** from the headset's Library > Unknown Sources.

## It runs on your own game files

No game's files are in this repository or the APK, and nothing is downloaded. Each engine runs on files from your
own copy of the game, imported once in the headset. Until you have imported one, the app opens on
**Settings > Game data**, which explains how.

- **Skate 3** (Xbox 360), converted from your own disc on a Windows PC:
  1. Run the [Skate 3 Rust Engine](https://github.com/SK8-ENGINE/skate-3-rust-engine/releases/tag/experimental)
     once on your ISO, and skate for a minute (this decodes its sound banks).
  2. Put `MAKE-QUEST-ZIP.bat` and `make-bundle.ps1` next to `skate3rust.exe` and double-click the `.bat`. It
     writes `Skate3-Quest.zip`.
  3. Copy the zip to the Quest's `Download/Skate3` folder.
  4. In the app: menu > Game data > Skate 3 > Import. It plays as soon as the import is done.

  Step by step: **[docs/IMPORT-SKATE3.md](docs/IMPORT-SKATE3.md)**.
- **THUG2** (PC): copy your installed game folder (the one holding `Data`), or a zip of it, to the Quest's
  `Download/THUG2` folder. Then menu > Game data > THUG2 > Import. It plays as soon as the import is done.
  More: **[docs/THUG2.md](docs/THUG2.md)**.

The first import asks for "All files access", so the app can read `Download`.

## In the headset

- **The menu** (tap the left menu button): your parks, the engine, AR settings, building a park, controls and game
  data. Each engine lists its own controls under Controls.
- **Build mode** (hold the left menu button for a second): place, move, scale and delete pieces. Pieces snap to
  table edges and to each other. Parks are saved and found again in the same room.
- **The room:** use your room scan (Space Setup), or free roam outside on one endless floor. Real things hide the
  skater (depth occlusion), and contact shadows ground it.
- **Roomtone (experimental):** lights the skater and your park from your real room's light, read from the
  passthrough cameras (Quest 3 / 3S). See [docs/room-lighting.md](docs/room-lighting.md).

## How it is built

The app is a generic runtime, and each game lives in its own repository behind one contract:

- **[buildar-engine-core](https://github.com/rhythmerc/buildar-engine-core)**: the contract between the runtime and
  an engine, as plain data. The runtime gives an engine the controllers, the stage and where to put the player. The
  engine steps on its own clock and hands back its models and poses, HUD draw lists and sounds. It also says what
  game files it needs. Each kind of game (a *mode*) adds its own part. Skating is the first.
- **The engines**, each Bevy-free in its own repository: `skate3-engine` in skate3-headless and `thug2-engine` in
  thug2-headless. The app pins each by revision.
- **The runtime** (this repository, `crates/buildar`): the room and occlusion, parks and the builder, menus, one
  shared renderer (skinned models, HUDs, contact shadows, Roomtone), spatial audio, the game data installer, and one
  driver that hosts any engine. No game's code is in it, apart from SK8, our own clean-room skating engine. SK8 is
  only offered to developers until it's ready.

Plans and notes are in [`docs/`](docs/); [docs/game-code-out-plan.md](docs/game-code-out-plan.md) covers the
contract and how the games moved out.

## Building

The headset app is `crates/buildar-quest` (`scripts/quest/build-apk.sh`), on the runtime crate `crates/buildar`.
The toolchain (Android NDK, `cargo-apk`, CMake and clang for THUG2's engine) is in
[scripts/quest/README.md](scripts/quest/README.md). The engines and the contract come from their repositories by
pinned revision. To work on one alongside the app, use the commented `[patch]` blocks in `Cargo.toml`.

Patched Bevy dependencies and their licenses are in [`vendor/`](vendor/).

## License and credits

GPL-3.0, the same as the upstream [Skate 3 Rust Engine](https://github.com/SK8-ENGINE/skate-3-rust-engine), and so
are skate3-headless and buildar-engine-core. thug2-headless is MIT. The clean skate sounds are CC0 (credits in
`crates/buildar/clean-audio/CREDITS.json`), and so are the park pieces' wood, concrete and brick textures
(ambientCG; `crates/buildar/park-textures/CREDITS.json`).

buildAR began as a fork of [andrewnakas/skate3-quest-ar](https://github.com/andrewnakas/skate3-quest-ar). It is an
unofficial project, not affiliated with EA, Activision or Neversoft.
