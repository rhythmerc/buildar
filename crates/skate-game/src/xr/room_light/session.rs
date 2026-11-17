//! Not for players: the headset's Roomtone session, recorded so that one
//! that looks wrong can be saved and replayed at a desk.
//!
//! Roomtone builds its estimate from everything it has been given since it
//! started (or last started over), so a recording from the moment something
//! looks wrong wouldn't make the same light. With "Record Roomtone for
//! captures" on (a developer row, `ArSettings::room_record`) when the app
//! starts, the capture's [`Room`] records every input it is given (the camera,
//! each frame and its depth image, the points the light is made for) into
//! `roomtone-session/` in [`crate::xr::XrFiles`], from Roomtone's start. The
//! setting is read at start only: turned on, it applies from the next start
//! (recording costs about 124 KB/s). Off at start, nothing is recorded and a
//! session left from before is deleted.
//!
//! "Save the Roomtone session" (the AR settings page's developer rows)
//! saves everything recorded so far into `captures/roomtone-<seconds>/`:
//!
//! - `session.rtone`: the recording (Roomtone's `Recorder` format);
//! - `note.json`: the app's commit, the room-light settings, and what the
//!   headset's Roomtone had made when it was saved (the light at the
//!   content's point, the points, its summary line).
//!
//! The recording carries on, and a later save has all of it again. Pull one
//! with
//!
//! ```text
//! adb pull /sdcard/Android/data/com.andrewnakas.skate3ar/files/captures/roomtone-<seconds>
//! ```
//!
//! and replay it on the desktop park (`SK8_ROOM_LIGHT=<folder>/session.rtone
//! cargo run --release -p skate-game --features xr -- --clean`; without `xr`
//! the variable is ignored), which logs whether the replay made the light
//! the note has, or with Roomtone's `tools/roomtone_replay <folder>/session.rtone`.
//!
//! The recording is of the room's light as the camera saw it (grids of
//! colour, not images) and the room's shape: like the occlusion capture, it
//! stays on the headset until it is pulled off over adb, and goes nowhere else.
//!
//! Its cost, on the headset's inputs (a 32x32 grid of four floats five times
//! a second, and with the room known a 64x64 16-bit depth image with each):
//! about 24.7 KB a frame, 124 KB/s, 7.4 MB a minute (16.4 KB a frame without
//! depth). Each frame is written and flushed by the capture's own thread as
//! it is added (about 40 microseconds a frame on a desktop), never the
//! game's; a save's copy has a thread of its own. A session that passes
//! [`CAP_BYTES`] starts over (Roomtone too), so the recording is always whole.
use super::RoomCapture;
use super::super::ar_settings::ArSettings;
use bevy::prelude::*;
use roomtone_sys::{Lighting, Recorder, Room};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A session this large starts over: about two and a half hours of frames.
pub(crate) const CAP_BYTES: u64 = 1 << 30;
/// Between looks at the session's size.
const CHECK_EVERY: Duration = Duration::from_secs(5);
/// The session's folder in the app's files.
const FOLDER: &str = "roomtone-session";

#[derive(Default)]
struct Shared {
    /// The recording so far: one file, then one more for each save.
    parts: Mutex<Vec<PathBuf>>,
    status: Mutex<String>,
}

/// The session being recorded, and how the last save went.
#[derive(Resource, Clone)]
pub(crate) struct RoomSession {
    dir: PathBuf,
    shared: Arc<Shared>,
}

impl RoomSession {
    pub(crate) fn new(files: &Path) -> Self {
        Self { dir: files.join(FOLDER), shared: Arc::default() }
    }

    fn say(&self, what: impl Into<String>) {
        *self.shared.status.lock().unwrap() = what.into();
    }

    /// For the settings page (empty: nothing to say).
    pub(crate) fn status(&self) -> String {
        self.shared.status.lock().unwrap().clone()
    }

    /// Whether a session is being recorded (the save button is shown).
    pub(crate) fn recording(&self) -> bool {
        !self.shared.parts.lock().unwrap().is_empty()
    }

    /// Starts `room` over and records it from there, in place of any session before.
    fn start(&self, room: &mut Room) -> Result<(), String> {
        self.stop(room);
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("No session folder: {e}"))?;
        let first = self.dir.join("0.rtone");
        let recorder = Recorder::create(&first).ok_or_else(|| format!("Can't write {}", first.display()))?;
        room.record(recorder);
        *self.shared.parts.lock().unwrap() = vec![first];
        Ok(())
    }

    /// Stops recording and deletes the session.
    fn stop(&self, room: &mut Room) {
        drop(room.stop_recording());
        self.shared.parts.lock().unwrap().clear();
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    /// The session's size on disk.
    fn bytes(&self) -> u64 {
        self.shared.parts.lock().unwrap().iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum()
    }

    /// Saves the session so far into `folder` (made here), with `note` (the
    /// caller's part of `note.json`) and what `room` has made now. The room
    /// carries on recording into a new part; the files are written by a
    /// thread of their own, which says how it went in [`RoomSession::status`].
    pub(crate) fn save(&self, room: &mut Room, folder: PathBuf, mut note: Value) -> Result<std::thread::JoinHandle<()>, String> {
        if !room.recording() {
            return Err("Roomtone isn't being recorded (turn \"Record Roomtone for captures\" on and restart).".into());
        }
        let parts = {
            let mut parts = self.shared.parts.lock().unwrap();
            let next = self.dir.join(format!("{}.rtone", parts.len()));
            let recorder = Recorder::create(&next).ok_or_else(|| format!("Can't write {}", next.display()))?;
            // (the part ended here is closed, whole, when dropped)
            drop(room.continue_recording(recorder));
            let saved = parts.clone();
            parts.push(next);
            saved
        };
        let points: Vec<[f32; 3]> = room.points().iter().map(|&p| p.into()).collect();
        note["roomtone"] = json!({
            "made": room.made(),
            "frames": room.frames(),
            "voxels": room.voxels(),
            "summary": room.summary(),
            "content_point": points.first(),
            "points": points,
            "lighting": room.light_at(0).as_ref().map(lighting_json),
        });
        let name = folder.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        self.say("Saving the Roomtone session...");
        let session = self.clone();
        std::thread::Builder::new()
            .name("sk8-roomtone-save".into())
            .spawn(move || {
                let said = match write(&folder, &parts, &note) {
                    Ok(bytes) => format!("Saved {name} in captures ({:.1} MB)", bytes as f64 / 1e6),
                    Err(e) => format!("The Roomtone session wasn't saved: {e}"),
                };
                info!("Room light: {said} ({}); adb pull {}", folder.display(), pull_path(&folder));
                session.say(said);
            })
            .map_err(|e| e.to_string())
    }
}

/// Where adb finds a capture folder (the app's files are external storage's).
fn pull_path(folder: &Path) -> String {
    let name = folder.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    format!("/sdcard/Android/data/com.andrewnakas.skate3ar/files/captures/{name}")
}

fn write(folder: &Path, parts: &[PathBuf], note: &Value) -> std::io::Result<u64> {
    std::fs::create_dir_all(folder)?;
    let mut note = note.clone();
    let bytes = roomtone_sys::join_recordings(parts, &folder.join("session.rtone"))?;
    note["bytes"] = json!(bytes);
    std::fs::write(folder.join("note.json"), serde_json::to_string_pretty(&note)?)?;
    Ok(bytes)
}

fn lighting_json(l: &Lighting) -> Value {
    json!({
        "level": l.level,
        "mean": l.mean,
        "sh": l.sh,
        "lights": l.lights.iter().map(|k| json!({
            "color": k.color,
            "dir": <[f32; 3]>::from(k.dir),
            "placed": k.placed(),
            "pos": <[f32; 3]>::from(k.pos),
            "range": k.range,
        })).collect::<Vec<_>>(),
    })
}

/// The app's part of a saved session's note: its commit (`SK8_COMMIT` when
/// built, which `scripts/quest/build-apk.sh` sets) and the room-light settings.
pub(crate) fn note(settings: &ArSettings) -> Value {
    json!({
        "kind": "roomtone_session",
        "commit": option_env!("SK8_COMMIT"),
        "saved_unix_s": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
        "room_light": settings.room_light,
        "room_light_strength": settings.room_light_strength,
    })
}

/// Records when the setting was on at start (`SK8_ROOM_RECORD` too), and
/// starts over when the session gets too large.
pub(super) fn keep_recording(
    session: Res<RoomSession>,
    capture: Res<RoomCapture>,
    settings: Res<ArSettings>,
    mut checked: Local<Option<Instant>>,
    mut failed: Local<bool>,
    mut at_start: Local<Option<bool>>,
) {
    // (read once: the settings are loaded before the first frame)
    let wanted = *at_start.get_or_insert_with(|| settings.room_record || std::env::var_os("SK8_ROOM_RECORD").is_some());
    let first = checked.is_none();
    let due = checked.is_none_or(|at| at.elapsed() >= CHECK_EVERY);
    // The capture may be busy on its own thread: next frame.
    let Ok(mut room) = capture.0.try_lock() else { return };
    // (a start that failed is tried again when due, not every frame)
    if (wanted == room.recording() || *failed) && !due {
        return;
    }
    if due {
        *checked = Some(Instant::now());
    }
    if !wanted {
        // (a session left from before is deleted too)
        if room.recording() || first {
            session.stop(&mut room);
        }
        return;
    }
    let too_big = room.recording() && session.bytes() > CAP_BYTES;
    if room.recording() && !too_big {
        return;
    }
    let started = session.start(&mut room);
    *failed = started.is_err();
    match started {
        Ok(()) if too_big => {
            info!("Room light: the recorded session passed {} MB: Roomtone started over", CAP_BYTES >> 20);
            session.say(format!("The session passed {} MB: Roomtone started over.", CAP_BYTES >> 20));
        }
        Ok(()) => info!("Room light: recording the Roomtone session (developer); Roomtone started over"),
        Err(why) => {
            warn!("Room light: no session recording: {why}");
            session.say(why);
        }
    }
}

/// Whether a replayed recording made the light its `note.json` (beside it)
/// says the headset had: a line for the log, `None` without a note.
pub(super) fn compare_with_note(room: &Room, recording: &Path) -> Option<String> {
    let note: Value = serde_json::from_str(&std::fs::read_to_string(recording.with_file_name("note.json")).ok()?).ok()?;
    let Some(saved) = note["roomtone"]["lighting"].as_object() else {
        return Some("the headset had made no light when it was saved".into());
    };
    let Some(made) = room.light_at(0) else { return Some("the replay made no light; the headset had".into()) };
    let made = lighting_json(&made);
    // Everything in it, as numbers in order (a desktop's floats may differ from the headset's in the last places).
    fn numbers(v: &Value, out: &mut Vec<f64>) {
        match v {
            Value::Number(n) => out.push(n.as_f64().unwrap_or(f64::NAN)),
            Value::Bool(b) => out.push(*b as u8 as f64),
            Value::Array(a) => a.iter().for_each(|v| numbers(v, out)),
            Value::Object(o) => o.values().for_each(|v| numbers(v, out)),
            _ => {}
        }
    }
    let (mut a, mut b) = (Vec::new(), Vec::new());
    numbers(&Value::Object(saved.clone()), &mut a);
    numbers(&made, &mut b);
    let worst = a.iter().zip(&b).map(|(a, b)| (a - b).abs() / a.abs().max(b.abs()).max(1e-3)).fold(0.0, f64::max);
    let level = |l: &Value| l["level"].as_f64().unwrap_or(f64::NAN);
    Some(if a.len() == b.len() && worst < 1e-3 {
        format!("the replay made the headset's light (level {:.3})", level(&made))
    } else {
        format!(
            "the replay's light differs from the headset's (level {:.3}, the headset's {:.3}; worst difference {:.1}%)",
            level(&made),
            level(&note["roomtone"]["lighting"]),
            worst * 100.0
        )
    })
}

#[cfg(test)]
mod tests {
    use super::super::{depth, feed::Feed};
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("sk8-roomtone-{name}-{}", std::process::id()))
    }

    /// The headset's way in: a camera, the points place_points sets, and
    /// frames added through the feed with depth of the room (the synthetic
    /// sweep's, each with a lamp), into the shared capture.
    fn add_frames(
        room: &mut Room,
        feed: &Feed,
        frames: std::ops::Range<usize>,
        all: &[(Vec<f32>, roomtone_sys::Pose, i64)],
        exposure_ms: f32,
    ) {
        for (cells, head, time_ns) in &all[frames] {
            let frame = roomtone_sys::Frame {
                cells,
                cols: 32,
                rows: 32,
                exposure_ms,
                iso: 100.0,
                head: *head,
                time_ns: *time_ns,
            };
            feed.add_frame(room, &frame);
        }
    }

    /// A session recorded through the app's feeding path, saved twice
    /// (the recording carries on after the first), replays into the light
    /// the room had made at each save, and the note says so.
    #[test]
    fn a_saved_session_replays_into_the_light_the_headset_made() {
        let (files, captures) = (temp("files"), temp("captures"));
        let _ = std::fs::remove_dir_all(&files);
        let _ = std::fs::remove_dir_all(&captures);
        let (min, max) = (Vec3::new(-2.0, 0.0, -2.5), Vec3::new(2.0, 2.5, 2.5));
        let eye = Vec3::new(0.3, 1.5, 0.2);
        let lamp = Vec3::new(0.35, 0.8, -0.5).normalize();
        // The sweep's frames, kept to be fed as the camera's thread would.
        let mut frames = Vec::new();
        roomtone_sys::synthetic::sweep_with(
            &mut Room::new(),
            |d| {
                let d = Vec3::from_array(d);
                if d.dot(lamp) > 0.1f32.cos() { [1.0; 3] } else { (Vec3::new(0.07, 0.055, 0.04) * (1.0 + 0.3 * d.y)).to_array() }
            },
            |_, frame| frames.push((frame.cells.to_vec(), roomtone_sys::Pose { position: eye.to_array().into(), ..frame.head }, frame.time_ns)),
        );
        let feeder = Feed::default();
        feeder.set_surfaces(Some(Arc::new(depth::Surfaces::new(depth::box_room(min, max)))));

        let session = RoomSession::new(&files);
        let mut room = Room::new();
        // Seen before the developer setting was on (a room ten times as
        // bright): dropped when it is, or the replay's light would differ.
        room.set_camera(&roomtone_sys::synthetic::camera());
        room.set_points(&[[0.0, 0.3, -0.5].into(), [1.0, 1.0, 1.0].into()]);
        add_frames(&mut room, &feeder, 0..60, &frames, 1.0);
        session.start(&mut room).unwrap();
        assert_eq!(room.frames(), 0, "started over");
        add_frames(&mut room, &feeder, 60..200, &frames, 10.0);
        // The park was placed: new points.
        room.set_points(&[[0.2, 0.3, -0.4].into(), [1.0, 1.0, -1.0].into(), [-1.0, 1.0, 1.0].into()]);
        add_frames(&mut room, &feeder, 200..240, &frames, 10.0);
        let first_light: Vec<Option<Lighting>> = (0..3).map(|i| room.light_at(i)).collect();
        let first = session.save(&mut room, captures.join("roomtone-1"), json!({ "commit": "test" })).unwrap();
        add_frames(&mut room, &feeder, 240..360, &frames, 10.0);
        // (300 frames since the start)
        let bytes = session.bytes();
        let second = session.save(&mut room, captures.join("roomtone-2"), json!({})).unwrap();
        let last_light: Vec<Option<Lighting>> = (0..3).map(|i| room.light_at(i)).collect();
        first.join().unwrap();
        second.join().unwrap();
        assert!(session.status().starts_with("Saved roomtone-"), "{}", session.status());
        assert!(room.recording(), "the recording carries on");
        assert!(room.voxels() > 0 && last_light.iter().all(Option::is_some), "{}", room.summary());

        for (name, light) in [("roomtone-1", &first_light), ("roomtone-2", &last_light)] {
            let recording = captures.join(name).join("session.rtone");
            let mut replayed = Room::new();
            assert!(replayed.replay(&recording));
            let replayed_light: Vec<Option<Lighting>> = (0..3).map(|i| replayed.light_at(i)).collect();
            assert_eq!(&replayed_light, light, "{name}");
            let said = compare_with_note(&replayed, &recording).unwrap();
            assert!(said.starts_with("the replay made the headset's light"), "{said}");
            // (another room's light is told apart)
            let other = Room::new();
            assert!(compare_with_note(&other, &recording).unwrap().starts_with("the replay made no light"));
        }
        let note: Value = serde_json::from_str(&std::fs::read_to_string(captures.join("roomtone-1/note.json")).unwrap()).unwrap();
        assert_eq!(note["commit"], "test");
        let content: Vec<f32> = note["roomtone"]["content_point"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
        assert_eq!(content, [0.2, 0.3, -0.4]);
        assert_eq!(note["roomtone"]["lighting"]["level"].as_f64().map(|v| v as f32), first_light[0].map(|l| l.level));

        // What it costs: the frames fed since the start, each with a depth image.
        let per_frame = bytes as f64 / 300.0;
        assert!((24_000.0..25_500.0).contains(&per_frame), "{per_frame} bytes a frame");

        // The setting off: no recording, and the session is gone.
        session.stop(&mut room);
        assert!(!room.recording() && !files.join(FOLDER).exists());
        let _ = std::fs::remove_dir_all(&files);
        let _ = std::fs::remove_dir_all(&captures);
    }
}
