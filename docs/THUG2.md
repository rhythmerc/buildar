# THUG2 in the headset

The headset app carries up to three skate engines and switches between them while it runs
(Settings > Engine):

| Engine  | What it is                                                     | Needs                             |
|---------|----------------------------------------------------------------|-----------------------------------|
| SK8     | the clean-room engine, made for this app                       | nothing                           |
| Skate 3 | the faithful port of Skate 3                                   | your imported Skate 3 data (IMPORT-SKATE3.md) |
| THUG2   | Tony Hawk's Underground 2's own engine, from thug2-decomp      | your own THUG2 PC game files      |

No game files are in this repository or in any APK. THUG2 only runs on files you copy from your
own copy of the game.

## Putting your THUG2 files on the headset

In the headset: copy your installed PC game folder (the one holding `Data`), or a zip of it, to
the Quest's `Download/THUG2` folder (USB, or any file app), then in the app **menu > Game data >
THUG2 > Import**. The first time, allow "All files access" and press it again. THUG2 plays as
soon as the import is done; the whole folder is copied (about 1.4 GB).

Or over adb, only the files the engine reads:

1. Install the app once, so its files folder exists.
2. With the headset connected over USB (developer mode on):

   ```sh
   scripts/quest/push-thug2.sh <your THUG2 folder>          # the ~20 MB the engine reads
   scripts/quest/push-thug2.sh <your THUG2 folder> --all    # or the whole Data folder (1.4 GB)
   ```

   `<your THUG2 folder>` is the installed game, the folder holding `Data`. The files go to
   `Android/data/com.andrewnakas.skate3ar/files/thug2/`.
3. In the app: Settings > Engine > THUG2.

The short list (`scripts/quest/thug2-files.txt`) is what a run of the engine reads: its scripts,
the empty developer level it starts in, what loads with the skater and its sound effects
(`Data/sounds/...`: without them the game plays no sounds). If a later version reads
more, `--all` always works.

## How it plays

- **The game's own skater.** Its physics, tricks, scripts and animation run as the game's own
  code. The skater is the model the game builds from your files (textures, clothes, board), posed
  by the game's animation every frame and drawn by the app.
- **The world is your room and your park.** Floors and ramps are ground. Room walls, furniture and
  box sides are walls: never ground, but they can be wallridden (hold B, the game's triangle, as you
  jump along one), as a THUG2 level's wallridable walls are. A quarter pipe's top is vert, so you go
  straight up and land back in. Rails and furniture edges are grindable.
- **Parks keep their engine.** A park is saved with the engine it's played with, and with THUG2's
  game speed. Opening it switches to that engine and puts that setting back. A
  park built for an engine that isn't installed asks first: Cancel, or play it with the current
  engine (it stays tagged with its own). The parks screen's **Engine** dropdown switches engines;
  ones that aren't installed are greyed out.
- **Switching engines** keeps the skater's spot: the new engine starts its skater where the last
  one was. Each engine carries on where it left off; THUG2 starts in under a second the first time
  it is picked.
- **The game waits** while a menu is open or build mode is on.
- **Too fast for your room?** THUG2's skater covers ground the way it does in the game's own big
  levels. Two settings help:
  - **Skater size** (Settings > Build a park, for every engine): a smaller skater makes the room bigger
    to the game, so the skater crosses it more slowly; ramps and rails stay the size you built them.
  - **Game speed** (Settings > THUG2, 40% - 100%, kept between launches): the game's clock. Sizes
    stay as they are, but jumps float.
- **The game's own sound effects.** Rolling, pops, landings, grinds and the rest are THUG2's own
  sounds from your files, at the level and pitch the game sets them (rolling rises with speed), and
  they come from the skater in the room. Music and voice streams aren't played yet.
  Settings > THUG2 > **Sound volume** (0 - 150%) sets their level.
- **The game's own HUD.** THUG2's score and special meter, its trick string with the points and
  multiplier, its balance meter and its messages, as the game draws them, rearranged for the room.
  The balance meter is at the skater, turned to you: a grind's or lip trick's arc over its head, a
  manual's upright meter beside it. It is as big as the skater is drawn, but never smaller than
  about a tenth of its distance from you, so it reads from across the room and with the world
  size up. The score panel and the trick string go by the controller Settings > AR picks (under
  "Trick and points readout"; the right by default): the score just over it, the trick string to
  its left, growing leftward. The corner readout the SK8 engine uses is hidden while THUG2 runs.
  The game draws its 2D only while it is seen (the readout on and no menu open).

### Controls

| Touch          | THUG2 (PS2 pad)                       |
|----------------|---------------------------------------|
| Either stick   | steer, push (up), trick directions    |
| A              | ollie (X)                             |
| B              | grind (triangle)                      |
| X              | flip trick (square)                   |
| Y              | grab trick (circle)                   |
| Left / right grip | L1 / R1: spin left / right         |
| Left / right trigger | L2 / R2: revert, nollie, switch |

Both sticks are the game's direction stick (the one pushed further counts), so either thumb can
leave its stick for a button: steer with the right stick while the left thumb flips or grabs (X / Y).

The menu button, build mode and placing the skater work as they do for the other engines.

## Building

The app's GitHub build (`crates/skate-quest`) has the `thug2` feature. It builds the engine with
CMake from two checkouts beside this repository (`crates/thug2-sys/build.rs`):

- `thug2-headless` (`THUG2_HEADLESS_DIR`, github.com/rhythmerc/thug2-headless): the harness that
  runs the decomp without a window, and the C API the app calls (`app/thug2_engine.h`).
- `thug2-decomp` (`THUG2_DECOMP_DIR`, github.com/rhythmerc/thug2-decomp, branch `linux-headless`):
  github.com/CanuckAussieKev/thug2-decomp with the Linux/Android patches.

```sh
git clone https://github.com/rhythmerc/thug2-headless ../thug2-headless
git clone -b linux-headless https://github.com/rhythmerc/thug2-decomp ../thug2-decomp
```

On the desktop: `cargo build -p skate-game --features xr,thug2`. Engine tests on your own files:
`THUG2_GAME_DIR=<folder holding Data> cargo test -p thug2-sys` and
`cargo test -p skate-game --features xr,retail,thug2 thug2::`.

`SK8_ENGINE_SCRIPT="8:thug2,16:sk8,24:skate3"` switches engines at those seconds after launch, to
test switching without the menu.

## How the switching works

`crates/skate-game/src/engines.rs`. Each engine's plugins are added inside `add_engine`:

- Their per-frame systems run only while that engine runs.
- Their start-up systems run the first time the engine is picked, not at launch.
- What they spawn is hidden, and their sounds paused, while another engine runs.

Nothing is unloaded, so switching back is instant. Skate 3's data, when imported, loads at
launch. Data imported while the app is running is there from the next launch.
