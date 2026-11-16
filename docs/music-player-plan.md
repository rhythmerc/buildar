# Planned: music import and an in-game music player

Noted 2026-11-15; not started.

## Import (installer)

The installer imports the soundtrack from the user's own game files, alongside the rest of each
game's data:

- **Skate 3**: its licensed tracks, from the user's Skate 3 data (IMPORT-SKATE3.md).
- **THUG2**: its soundtrack, from the user's THUG2 PC folder (THUG2.md).

As with everything else, no music ships in the repository or the APK.

## Player (in game)

- Plays the imported tracks in shuffle.
- **Library**: by default the active engine's (THUG2's tracks while THUG2 runs, Skate 3's while
  Skate 3 or SK8 runs). The player can pick a library instead: THUG2, Skate 3, or both together.
- Switching engines switches the library when it's left on the default.
