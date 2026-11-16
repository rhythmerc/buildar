# Co-located multiplayer: shared frame and shared room (plan)

Two or more Quest 3 headsets in one room skating the same park: everyone needs the same spatial frame
(so pieces and skaters line up) and the same room geometry (the room scan is collision, so every
engine's physics must see identical surfaces). The app is sideloaded (not store-bound), so this has
to work on the LAN, offline, without Meta's entitlement. Part of the larger multiplayer-sessions
effort. Research only so far (2026-11-16); nothing here is built yet. Accuracy figures marked "est."
are engineering estimates.

## Meta's own sharing is not usable

- **Shared Spatial Anchors / Group Sharing** need `IMPORT_EXPORT_IOT_MAP_DATA`, Enhanced Spatial
  Services, internet and Meta's cloud. Save/share fail with "package is not trusted" unless the app is
  registered on the developer dashboard. (No explicit doc says group sharing hard-fails for an
  unregistered package; the troubleshooting page implies it.)
- **Colocation Discovery** works over Bluetooth, but only for a verified developer team, its test
  users, or a non-production release channel.
- Neither works offline, and neither is usable for a sideloaded app's end users.

## Alignment (the shared frame)

Ranked:

1. **Floor-spot touch alignment (build first).** Each player rests a controller on two marked floor
   spots (tape, shoes), one after the other, so nobody has to touch at the same moment. A 2D fit on
   the floor plane (Kabsch over x, z) gives yaw and position; the floor gives height. Est. 1-2 cm and
   about 1 degree with spots 1.5 m apart. No new permissions, offline, small effort.
2. **Automatic refinement: match the joiner's scan to the host's mesh.** The host's room mesh is sent
   anyway (below). The joiner aligns its own Space Setup mesh, or a few seconds of live depth points
   from `XR_META_environment_depth` (already used in `xr/occlusion.rs`), against it with
   point-to-plane ICP: yaw and position only, since gravity is shared, seeded by step 1.
   - Accuracy: est. 1-3 cm and under 1 degree in a furnished room.
   - Weakness: empty or symmetric rectangular rooms are ambiguous; the touch seed resolves that.
   - Doubles as a continuous drift and validity check. Medium effort, pure geometry, testable off
     device like `xr/mesh_planes.rs`.
3. **Printed QR code on the floor, through `XR_EXT_spatial_marker_tracking` (later, convenience).**
   Gives a 6DoF pose; reported working on Quest 3 from Horizon OS v83 (QR only), with a report of it
   breaking on a later build (Aug 2026). The bindings in the build (openxr-sys 0.13.1) define it.
   Meta's own QR tracking (MRUK) needs only `USE_SCENE`, which the app has. Est. about 1 cm and
   1-2 degrees of yaw for an A4 code; a second code improves yaw. Fallback: our own AprilTag
   detection through Camera2 (`HEADSET_CAMERA`, already requested; lens intrinsics and pose already
   read in `xr/room_light/camera.rs`). Medium-high effort.
4. **Rejected:** a marker shown on another headset's panel (exposure, moving target); our own ORB
   relocalization (high effort, beaten by mesh ICP).

### How the frame fits the existing code

- The shared frame **S** is a host "room anchor", made with the existing `Anchors::make` and local
  save path (`xr/build/anchor.rs`). Its identity is that anchor's UUID.
- Each joiner makes its own local anchor and stores where S sits relative to it: exactly the shape of
  `AnchorRecord {uuid, offset, turn}` and the math of `ParkPose::by_anchor/record`
  (`xr/build/parks.rs`). Yaw-only, as the anchors already are.
- The local anchor is re-located every frame, which absorbs tracking recentres and the floor shift
  between sessions. On restart, a found anchor brings the frame back automatically; the ICP residual
  confirms it (over about 3 cm: ask to re-align).
- Parks travel as `ParkFile` JSON with their pose relative to S, without the host's anchor records;
  joiners attach their own `AnchorRecord`.
- World scale `k` (`AnchorRecord.scale`, `xr/rig.rs`) must be identical on every headset. With the
  same park pose and scale everywhere, the existing game-world snapshots (`skate-net` packed) need no
  change.

## Shared room geometry (collision)

- **Rule: the room is fixed for the session.** Once the host shares its scan, it does not change
  until the session ends. The host is barred from re-scanning during a live session.
- **The host's scan wins.** Every engine must see identical surfaces, or remote skaters float or
  sink. No merging (it duplicates surfaces and ledges, and peers diverge).
- **What is sent:** the inputs to the collision build, not `ArWorld` (which is already in post-rig
  world space and mixes in the pieces): `SceneShapes` (plane and box poses, half extents, labels) and
  the `LocatedMesh` vertices and indices (`xr/planes.rs`), all in S. Each receiver maps S into its own
  tracking space and runs the same `rebuild_collision` (`xr/real_world.rs`), so quads, triangles,
  grind edges and sunk-piece holes come out identical everywhere. `MeshPlanes` is recomputed from the
  mesh deterministically.
- **Size** (about 5k triangles, 2.5-3k vertices): about 96 KB raw; with 16-bit quantised positions
  (0.5 mm over 32 m, as `packed.rs`), delta-coded u16 indices and zstd, about 25-35 KB. Shapes about
  2 KB. No decimation needed (if ever, on the host only).
- **Sent once**, with its revision and hash, when a player joins (late joiners get the same blob).
- **Enforcing the rule on the host:** snapshot `SceneShapes` and `RoomMesh` into a frozen "session
  room" when the session starts, and build collision only from that (today `rebuild_collision`
  rebuilds on any scene revision change: the session room's revision replaces it). The local refresh
  may keep running for occlusion and visuals. If the host starts Space Setup anyway, or its room
  anchor is lost: pause or end the session and ask everyone to re-align; don't re-sync.
- **Joiners** replace their own collision with the host's, and keep their own scan and live depth
  only for occlusion (necessarily per device), visuals, room light and the drift check. Visible cost:
  where the scans differ, the skater can look slightly sunk into, or floating over, what that player
  sees.
- **Disagreement check** after alignment: compare floor heights and wall-plane distances. Under 2 cm,
  use the host's silently; over 5 cm (or a poor ICP fit), show "re-align or re-scan".
- **Sharing the mesh helps alignment:** the host's mesh is a free reference map, which makes ICP the
  default refinement and continuous check, and QR a convenience rather than a necessity. With the room
  frozen, a rising residual always means drift or a lost anchor, never a changed room.

## Park edits during a session

- The host decides. A joiner sends an edit request; the host applies it to its park file and sends
  the result to everyone with an increasing park revision.
- `PieceEdit {park_rev, op: Add | Move | Remove | Scale, piece id, Placed}`, pose in the park's
  frame: tens of bytes, over the reliable stream. A joiner that falls behind gets the whole
  `ParkFile` again.
- Each headset applies it to its placed pieces and rebuilds collision at the existing safe point (not
  grinding, no piece being dragged). Sunk-piece holes and seating are rebuilt locally from the pieces
  against the frozen room, so they match everywhere.
- A piece being dragged may show to others as a temporary pose; it commits (and changes collision)
  only when dropped, as `placed.busy()` works now. Piece edits never touch anchors.

## LAN protocol

- **Discovery:** the host broadcasts a UDP beacon once a second: magic, protocol version, session id,
  host name, room UUID, room revision, game port. mDNS (`_skate-ar._udp`) optional.
- **Connection:** the existing `Direct` UDP transport (`multiplayer/transport.rs`, IPv4 only) and the
  star lobby.
- **Bulk:** room geometry and park files over the reliable blob stream (`skate-net/blob.rs`, already
  carries GLBs up to 64 MiB).
- **Control messages:** `Align {method, points in S}`, `AlignResult {theta, t, residual}`,
  `Room {rev, hash}`, `Park {rev}`, `PieceEdit`.
- **Manifest gap:** `crates/skate-quest/Cargo.toml` requests no `android.permission.INTERNET`.
  Receiving broadcast or mDNS on Android also needs `CHANGE_WIFI_MULTICAST_STATE` and a MulticastLock
  through JNI.

## Suggested build order

1. Missing permissions (`INTERNET`, multicast) and the LAN beacon, with a type-in-the-IP fallback.
2. Floor-spot touch alignment, and the shared frame on the existing anchors.
3. Frozen session room on the host; room blob sent at join; joiners build collision from it.
4. Parks over the session, then host-authoritative piece edits.
5. Mesh ICP refinement and the continuous drift check.
6. QR code alignment (when the platform extension is reliable).

## Open risks

- `XR_EXT_spatial_marker_tracking` is new on Horizon OS (v83+, QR only), with reports of regressions.
- ICP can lock onto a symmetric room the wrong way round if the touch seed is bad.
- Furniture moved between the two scans biases the fit: ICP needs outlier rejection.
- Furniture moved during a session is not reflected in collision; the UI should say so, and the drift
  check should flag large local differences.
- The joiner skates the host's surfaces, not its own: they can disagree visually.
- Some routers isolate Wi-Fi clients from each other: the beacon needs a manual-IP fallback.
- A boundary reset or lost anchor on the host invalidates S: pause and re-align.
- Sending the room mesh shares spatial data: make it opt-in, with the players' consent.

## Sources

- [Meta: OpenXR spatial anchors developer setup](https://developers.meta.com/horizon/documentation/native/android/openxr-spatial-anchors-developer-setup/)
- [Meta: Shared Spatial Anchors troubleshooting](https://developers.meta.com/horizon/documentation/native/android/openxr-ssa-ts/)
- [Meta: Unity Shared Spatial Anchors](https://developers.meta.com/horizon/documentation/unity/unity-shared-spatial-anchors/)
- [Meta: Unity Colocation Discovery](https://developers.meta.com/horizon/documentation/unity/unity-colocation-discovery)
- [Meta: Unreal Colocation Discovery](https://developers.meta.com/vr/documentation/unreal/unreal-colocation-discovery/)
- [Meta: MRUK QR code sample](https://developers.meta.com/horizon/documentation/unity/unity-sample-mruk-qr-code)
- [Godot forum: QR tracking through OpenXR spatial entities on Quest 3](https://forum.godotengine.org/t/qrcode-tracking-via-openxr-spatial-entities-extension-on-meta-quest-3/123395)
- [Meta community forum: OpenXR spatial markers seem broken](https://communityforums.atmeta.com/questions-and-discussions-26/openxr-spatial-markers-seem-broken-206-0-0-109-432-179184)
- [Android XR: XR_EXT_spatial_marker_tracking](https://developer.android.com/develop/xr/openxr/extensions/XR_EXT_spatial_marker_tracking)
- [UploadVR: OpenXR spatial entities extensions](https://www.uploadvr.com/openxr-spatial-entities-extensions/)
- [UploadVR: MRUK update tracks QR codes](https://uploadvr.com/quest-3-mruk-update-decode-track-qr-codes-unity)
- [QuestCameraKit (ZXing QR codes on Quest)](https://www.sourcepulse.org/projects/2336119)
