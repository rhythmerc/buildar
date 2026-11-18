#!/usr/bin/env bash
# Copy your own THUG2 PC game files to the headset, for the THUG2 engine (see docs/THUG2.md). Run after the APK is
# installed once (so the app's files folder exists). Nothing here is ever committed or bundled.
#   scripts/quest/push-thug2.sh <game-folder> [--all]
# <game-folder>  the installed (or extracted) game: the folder holding Data/
# --all          the whole Data folder (about 1.4 GB) instead of the files the engine reads (thug2-files.txt, ~20 MB)
set -euo pipefail
GAME="${1:?usage: push-thug2.sh <game-folder> [--all]}"
HERE="$(cd "$(dirname "$0")" && pwd)"
PKG=${PKG:-io.github.rhythmerc.buildar}
DEST=/sdcard/Android/data/$PKG/files/thug2
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
if [ "${2:-}" = "--all" ]; then
  cp -r "$GAME/Data" "$STAGE/Data"
else
  # (the game's names are matched whatever their case, as the game does)
  while IFS= read -r file; do
    [ -n "$file" ] || continue
    found="$(cd "$GAME" && find . -ipath "./$file" -print -quit)"
    [ -n "$found" ] || { echo "missing in $GAME: $file" >&2; exit 1; }
    mkdir -p "$STAGE/$(dirname "$file")"
    cp "$GAME/$found" "$STAGE/$file"
  done < "$HERE/thug2-files.txt"
fi
du -sh "$STAGE"
# (Android won't let adb push create folders inside another app's files: make them first)
(cd "$STAGE" && find . -type d | sed 's|^\./||;s|^\.$||') | while IFS= read -r d; do
  printf 'mkdir -p "%s/%s"\n' "$DEST" "$d"
done > "$STAGE.mkdirs.sh"
adb push "$STAGE.mkdirs.sh" /data/local/tmp/thug2-mkdirs.sh >/dev/null
adb shell sh /data/local/tmp/thug2-mkdirs.sh
adb shell rm /data/local/tmp/thug2-mkdirs.sh
rm -f "$STAGE.mkdirs.sh"
adb push --sync "$STAGE/." "$DEST/"
echo "Pushed to $DEST: pick THUG2 in Settings > Engine"
