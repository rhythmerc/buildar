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
# (the disc's folders made first: adb push can't make folders under Android/data on the headset, the shell can)
(cd "$DISC" && find sys files $( [ -d "$DISC/wii_bootstrap" ] && echo wii_bootstrap ) -type d) | sed "s|^|$DEST/|" | xargs adb shell mkdir -p
adb push --sync "$DISC/sys" "$DEST/"
adb push --sync "$DISC/files" "$DEST/"
# (the runtime's own files the library looks for beside the disc - the DSP's coefficients, the NAND's bootstrap: from
# WiiCompiled's build output, copied into the disc folder)
for f in dsp_coef.bin wii_bootstrap; do
  if [ -e "$DISC/$f" ]; then
    adb push --sync "$DISC/$f" "$DEST/"
  else
    echo "warning: no $f in $DISC (copy it there from WiiCompiled's build output): the game won't boot without it" >&2
  fi
done
adb push --sync "$LIB" "$DEST/libmkw_engine.so"
echo "Pushed to $DEST: pick Mario Kart Wii in the app (Settings > Engine), or open a track"
