# Repository content

Private runtime assets, extracted maps, custom animation clips, build output,
settings and logs stay local. They are excluded by `.gitignore`.

The project-owned binary test fixtures live in the
[skate3-headless](https://github.com/rhythmerc/skate3-headless) repo, generated without game assets:
`skate-data/tests/fixtures/retail-collision.rwcmset` (`python tools/make_collision_fixture.py` here writes
into a sibling checkout of it) and `sim/tests/fixtures/format-demo.skate`.

Run these commands from the repository root. Both generators reproduced the
tracked fixtures byte for byte during publication preparation.

The README artwork in `docs/images/skating-crab.png` was supplied by the project
owner for publication. Other binary resources belong to the vendored Bevy dependencies, whose license
files remain alongside their source. Do not add extracted game content to
these directories or force-add ignored private files.

The pre-publication audit inspected all reachable historical blobs for private
paths, matches against local private-file hashes, unexpected binary files and
common credential patterns. No matches requiring removal were found.
