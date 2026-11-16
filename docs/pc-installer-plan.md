# PC installer plan (parked)

Status: **parked** (2026-11-15). A rough plan, not a spec. The full write-up with wireframes is
[pc-installer-plan.html](pc-installer-plan.html) (also published privately at
https://claude.ai/artifact/EHc8ALfCQYXG5pyRutWzXD).

## What it is for

One desktop app for Windows and Linux that:

- installs the Quest app on the headset over USB (adb);
- lists the three engines - sk8 (built in, needs nothing), Skate 3, THUG2 - with their state;
- turns the user's own game copy into a ready bundle on the PC and pushes it to the headset.

All three engines ship in the one Quest app. Skate 3 and THUG2 say "Needs your game files" until the user
imports their own copy. No game files ever ship in the installer or the app.

## Recommendations

- **Toolkit:** Rust with egui/eframe: one native binary per OS, no web view (Tauri's WebKitGTK shows blank windows on
  NVIDIA Linux), and it can call `skate-data` directly. Tauri is the runner-up.
- **Data path:** convert on the PC, then copy the finished folder plus a `bundle.json` manifest (format version,
  per-file size and sha256) straight into the app's files folder on the headset. Copying is file by file, so a pulled
  cable resumes, and the headset never needs room for a zip and its unzipped copy. Finished bundles are kept on the PC
  for quick reinstalls.
- **Skate 3:** reuse the existing desktop conversion (`tools/setup.py`, `tools/prepare_assets.py`); add a Rust step
  that decodes every sound bank up front (today they decode lazily while playing, hence "skate for a minute"). The
  fast path - finding an existing skate3rust conversion - retires `MAKE-QUEST-ZIP.bat` and the manual copy.
- **THUG2:** accept an installed game folder, or BIN/CUE / ISO disc images: read the discs, read the installer's MSI
  tables, unpack its cabinets and rebuild `Data/` in Rust (what `7z` + `msiextract` did by hand). Validate against a
  known file list; detect the patch version.
- **On the headset:** v1 uses today's layout (`assets/`, the PCM cache, `use-retail-data`), so the app needs no
  changes; later `engines/<name>/` with its own `bundle.json` and an `engine` file picking the next engine.
- **Packaging:** NSIS per-user installer on Windows (no admin); AppImage on Linux (Flatpak's sandbox blocks USB
  access to the headset). A udev rule for Meta's USB vendor ID on Linux; Meta's ADB driver on Windows.

## Milestones (one developer, rough)

| | What | Effort |
|---|---|---|
| M0 | eframe shell, adb detection, headset states, udev / driver checks | 2-3 days |
| M1 | install / update the Quest app | 1 week |
| M2 | Skate 3 from an existing conversion, audio pre-decode, resumable push | 1-1.5 weeks |
| M3 | Skate 3 straight from the ISO (incl. a Linux build of the converter) | 2-3 weeks |
| M4 | THUG2 data import (folder and disc images) | 1.5-2 weeks |
| M5 | multi-engine layout on the headset, in-app Engine menu, THUG2 core in the release APK | 1 week + engine port |
| M6 | self-update, wireless adb, signing, polish | 1-2 weeks |

M1 + M2 (about 2.5 weeks) is the smallest useful slice.

## Open questions

1. Should the installer also install the desktop game (Windows only today)?
2. Keep the `com.andrewnakas.skate3ar` package (existing installs and data carry over) and retire the `sk8ar` store
   build?
3. Is Skate 3 conversion straight from an ISO on Linux needed in v1?
4. Build the THUG2 C++ core for Android inside the main workspace, or ship a prebuilt library?
5. Whole THUG2 `Data/` (1.4 GB) on the Quest, or only the files the engine opens? Which THUG2 versions must work?
6. Keep finished bundles on the PC (5-7 GB) by default?
7. Installer builds on the fork's GitHub releases? A Windows code-signing certificate?
8. Wireless headset connection in v1?
