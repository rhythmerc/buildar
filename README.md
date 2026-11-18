<p align="center">
  <img src="docs/images/skating-crab.png" alt="Rust crab riding a skateboard" width="480">
</p>

# Skate 3 AR for Meta Quest 3 (alpha)

A skater in your real room in passthrough AR on Quest 3: ride your floor, tables and furniture,
and build a park of ramps, rails, ledges, quarter pipes and stairs with the Touch controllers.
Ollies and flip tricks, grinds and slides, manuals, grabs, body flips, spins and big airs.

**[Download the latest APK](https://github.com/andrewnakas/skate3-quest-ar/releases/latest)**
and install it with `adb install -r skate3ar.apk` (or SideQuest). Open **Skate 3 AR** from the
headset's Library > Unknown Sources.

**It runs on your own game files.** Bring Skate 3 (Xbox 360) or THUG2 (PC) from your own copy;
until then the app opens on Settings > Game data, which says how. (Our own clean-room engine,
SK8, is not offered yet: too rough. Developers can turn it on.)

**Skate 3** (retail skater, animations, controls and sounds), converted from your own disc.
Four steps, one Windows PC:

1. Run the [Skate 3 Rust Engine](https://github.com/SK8-ENGINE/skate-3-rust-engine/releases/tag/experimental)
   once on your ISO and skate for a minute.
2. Put `MAKE-QUEST-ZIP.bat` + `make-bundle.ps1` (from the release) next to `skate3rust.exe`
   and double-click the `.bat`: it writes `Skate3-Quest.zip`.
3. Copy the zip to the Quest's `Download/Skate3` folder.
4. In the app: menu > Game data > Skate 3 > Import. It plays as soon as the import is done.

**THUG2:** copy your installed PC game folder (the one holding `Data`) to the Quest's
`Download/THUG2` folder, then menu > Game data > THUG2 > Import; it plays at once
([docs/THUG2.md](docs/THUG2.md)).

Step-by-step with pictures of where everything is: **[docs/IMPORT-SKATE3.md](docs/IMPORT-SKATE3.md)**.
No EA assets are in this repo or the APK.

### Controls

| Touch | |
|---|---|
| Right stick | Hold down to crouch, flick up to ollie; flick directions for flip tricks |
| Left stick | Steer; in the air, spin; pushed back to a diagonal: powerslide |
| A / X | Push / mongo push (tap a light push, hold a full one) |
| B / Y | Brake / step off the board |
| Triggers (air) | Grab; **grab + left stick forward/back: front flip / back flip** (add a spin for flip 360s, 540s) |
| Right stick on a rail | Up/down: nosegrind / 5-0; right out to a side and held: turns into crooked, salad, feeble, smith, boardslide, lipslide |
| Left menu (tap / hold 1 s) | Menu / build mode (A places, B deletes, right stick turns and scales the piece) |

More in [scripts/quest/README.md](scripts/quest/README.md) and the in-app How to play page.

**Roomtone room lighting (experimental):** menu > AR settings > Roomtone room lighting lights the skater and your
park by your real room, estimated from the passthrough cameras (Quest 3 / 3S). See
[docs/room-lighting.md](docs/room-lighting.md).

**Handing off to Snap Spectacles?** Start with [docs/HANDOFF.md](docs/HANDOFF.md): how this
maps onto the C++ port [skatecore-cpp](https://github.com/andrewnakas/skatecore-cpp).

## Building

The headset app is `crates/skate-quest` (`scripts/quest/build-apk.sh`), on the game crate
`crates/skate-game`. The Skate 3 simulation comes from
[skate3-headless](https://github.com/rhythmerc/skate3-headless). See
[scripts/quest/README.md](scripts/quest/README.md) for the toolchain.

Implementation notes are in [`docs/`](docs/). Patched Bevy dependencies and
their licenses are in [`vendor/`](vendor/). This is an unofficial project,
not affiliated with EA.

## License

GPL-3.0, the same as the upstream
[Skate 3 Rust Engine](https://github.com/SK8-ENGINE/skate-3-rust-engine).
Clean skate sounds are CC0 (credits in `crates/skate-game/clean-audio/CREDITS.json`), and so are
the park pieces' wood, concrete and brick textures (ambientCG; `crates/skate-game/park-textures/CREDITS.json`).
