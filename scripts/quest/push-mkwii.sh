#!/usr/bin/env bash
# Copy your own Mario Kart Wii (extracted disc) and the engine library built from it to the headset, for the Mario
# Kart Wii engine. Run after the APK is installed once (so the app's files folder exists). Nothing here is ever
# committed or bundled.
#   scripts/quest/push-mkwii.sh <extracted-disc> <libmkw_engine.so>
# <extracted-disc>     nodtool's extraction of your disc: the folder holding sys/ and files/
# <libmkw_engine.so>   the engine library built from that disc for Android arm64 (WiiCompiled, MKW_HEADLESS_ENGINE),
#                      stripped
set -euo pipefail
DISC="${1:?usage: push-mkwii.sh <extracted-disc> <libmkw_engine.so>}"
LIB="${2:?usage: push-mkwii.sh <extracted-disc> <libmkw_engine.so>}"
PKG=${PKG:-io.github.rhythmerc.buildar}
DEST=/sdcard/Android/data/$PKG/files/mkwii
[ -f "$DISC/sys/main.dol" ] || { echo "no sys/main.dol in $DISC" >&2; exit 1; }
adb shell mkdir -p "$DEST"
adb push --sync "$DISC/sys" "$DEST/"
adb push --sync "$DISC/files" "$DEST/"
adb push --sync "$LIB" "$DEST/libmkw_engine.so"
echo "Pushed to $DEST: pick Mario Kart Wii in the app (Settings > Engine), or open a track"
