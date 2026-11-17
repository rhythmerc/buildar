//! Safe Rust wrapper for Roomtone (C API in `vendor/include/roomtone/roomtone.h`,
//! vendored from <https://github.com/rhythmerc/roomtone>, MIT): room light
//! estimation for mixed reality.
//!
//! Meta Quest has no light estimation API. Roomtone makes the estimate from
//! what the headset does give: passthrough camera frames reduced to a small
//! grid of linear colour, each with its exposure and the head's pose, and
//! (optionally) depth. A [`Room`] takes those and answers with the
//! [`Lighting`] at a point: L2 spherical harmonics for the soft light, up to
//! two key lights, and a level that matches passthrough's brightness.
//!
//! Coordinates are OpenXR tracking space: metres, +Y up. A `Room` is `Send`
//! but not `Sync`: put it behind a mutex to share it.
//!
//! A room can also write what it is given to a [`Recorder`] ([`Room::record`]),
//! which [`Room::replay`] (or Roomtone's `tools/roomtone_replay`) plays back.
use std::ffi::{CString, c_char, c_int};
use std::io::{Read, Write};
use std::path::Path;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl From<[f32; 3]> for Vec3 {
    fn from(v: [f32; 3]) -> Self {
        Self { x: v[0], y: v[1], z: v[2] }
    }
}

impl From<Vec3> for [f32; 3] {
    fn from(v: Vec3) -> Self {
        [v.x, v.y, v.z]
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Quat {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    pub orientation: Quat,
    pub position: Vec3,
}

/// The camera as Android's Camera2 describes it (Meta's Passthrough Camera API).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Camera {
    /// `LENS_POSE_ROTATION`: turns the head's axes into the camera's.
    pub lens_rotation: Quat,
    /// `LENS_INTRINSIC_CALIBRATION` (fx, fy, cx, cy), in the pixels of the
    /// sensor array (`SENSOR_INFO_PRE_CORRECTION_ACTIVE_ARRAY_SIZE`).
    pub fx: f32,
    pub fy: f32,
    pub cx: f32,
    pub cy: f32,
    pub sensor_width: i32,
    pub sensor_height: i32,
    /// The stream: the whole sensor array, scaled.
    pub width: i32,
    pub height: i32,
}

/// A camera frame reduced to a grid of `cols` x `rows` cells, row-major from
/// the top: each cell's mean linear r, g, b (0-1 as the camera gave them)
/// and the share of its pixels that clipped.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    /// `cols * rows * 4` values.
    pub cells: &'a [f32],
    pub cols: usize,
    pub rows: usize,
    /// A frame without an exposure and a sensitivity is left out.
    pub exposure_ms: f32,
    pub iso: f32,
    /// The head when the frame was taken.
    pub head: Pose,
    /// A steady clock.
    pub time_ns: i64,
}

/// A depth image (`XR_META_environment_depth`'s convention): OpenGL depth
/// (0-1 over 65535, rows from the bottom), `near_z / (1 - d)` along the
/// view's axis with an infinite far plane (`far_z` 0) or a far plane.
#[derive(Clone, Copy, Debug)]
pub struct DepthImage<'a> {
    /// `stride * height` values at least.
    pub depth: &'a [u16],
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub near_z: f32,
    pub far_z: f32,
    pub tan_left: f32,
    pub tan_right: f32,
    pub tan_up: f32,
    pub tan_down: f32,
    pub view: Pose,
    /// The steady clock frames use; frames more than 0.5 s later don't use it.
    pub time_ns: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Light {
    /// Irradiance over pi, in the SH's units (an up-facing surface in the
    /// reference gets 1 in all).
    pub color: [f32; 3],
    /// Toward the light, from the point asked about.
    pub dir: Vec3,
    placed: i32,
    /// Where it is in the room, when [`Light::placed`].
    pub pos: Vec3,
    /// A placed light's distance from the point, where `color` applies: a
    /// surface elsewhere scales it by `range^2 / d^2`.
    pub range: f32,
}

impl Light {
    /// Whether depth gave the light a place (`pos`, `range`) in the room.
    pub fn placed(&self) -> bool {
        self.placed != 0
    }

    pub fn lit(&self) -> bool {
        self.color.iter().any(|c| *c > 0.0)
    }
}

/// The light at a point.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Lighting {
    /// SH L2 irradiance over pi, each coefficient times its basis constant:
    /// evaluate the bare polynomials 1, y, z, x, xy, yz, 3z^2 - 1, xz,
    /// x^2 - y^2 at the unit normal (tracking space). See [`Lighting::irradiance`].
    pub sh: [[f32; 3]; 9],
    pub lights: [Light; 2],
    /// The reference's mean light over every direction, without its lights'
    /// cores (each one's brightest 17 degrees): the brightness's reference.
    pub mean: f32,
    /// The reference's brightness as passthrough shows it.
    pub level: f32,
}

impl Lighting {
    /// The soft light (the SH, without the key lights) on a surface facing
    /// `normal` (unit, tracking space): irradiance over pi.
    pub fn irradiance(&self, normal: [f32; 3]) -> [f32; 3] {
        let [x, y, z] = normal;
        let basis = [1.0, y, z, x, x * y, y * z, 3.0 * z * z - 1.0, x * z, x * x - y * y];
        let mut out = [0.0f32; 3];
        for (row, b) in self.sh.iter().zip(basis) {
            for c in 0..3 {
                out[c] += row[c] * b;
            }
        }
        out.map(|v| v.max(0.0))
    }
}

/// A bright spot the capture has seen (a fixture, a window).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spot {
    /// Toward it, from where the head was (unit, tracking space).
    pub dir: Vec3,
    /// Its brightest cell's brightness, times the room's mean.
    pub brightness: f32,
    clipped: i32,
}

impl Spot {
    pub fn clipped(&self) -> bool {
        self.clipped != 0
    }
}

/// What a frame did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameResult {
    /// False: no exposure, no camera, an empty grid or a black image.
    pub used: bool,
    /// Left out: the image was black (a covered or stopped camera).
    pub black: bool,
    /// The room changed (a light switched): the capture started over.
    pub restarted: bool,
    /// The frame against the capture, by its median.
    pub change: f32,
}

mod ffi {
    use super::{Camera, Light, Lighting, Pose, Spot, Vec3};
    use std::ffi::{c_char, c_int};

    #[repr(C)]
    pub struct Room {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct Recorder {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct Frame {
        pub cells: *const f32,
        pub cols: i32,
        pub rows: i32,
        pub exposure_ms: f32,
        pub iso: f32,
        pub head: Pose,
        pub time_ns: i64,
    }

    #[repr(C)]
    pub struct DepthImage {
        pub depth: *const u16,
        pub width: i32,
        pub height: i32,
        pub stride: i32,
        pub near_z: f32,
        pub far_z: f32,
        pub tan_left: f32,
        pub tan_right: f32,
        pub tan_up: f32,
        pub tan_down: f32,
        pub view: Pose,
        pub time_ns: i64,
    }

    #[repr(C)]
    pub struct FrameResult {
        pub used: i32,
        pub restarted: i32,
        pub change: f32,
        pub black: i32,
    }

    // The layouts roomtone.h declares.
    const _: () = assert!(size_of::<Light>() == 44 && size_of::<Lighting>() == 204 && size_of::<Spot>() == 20);

    unsafe extern "C" {
        pub fn roomtone_room_new() -> *mut Room;
        pub fn roomtone_room_free(room: *mut Room);
        pub fn roomtone_room_reset(room: *mut Room);
        pub fn roomtone_room_set_camera(room: *mut Room, camera: *const Camera);
        pub fn roomtone_room_has_camera(room: *const Room) -> c_int;
        pub fn roomtone_room_add_frame(room: *mut Room, frame: *const Frame) -> FrameResult;
        pub fn roomtone_room_add_hit(room: *mut Room, origin: Vec3, point: Vec3);
        pub fn roomtone_room_add_depth(room: *mut Room, image: *const DepthImage);
        pub fn roomtone_room_moved(room: *mut Room, p0: Vec3, yaw0: f32, p1: Vec3, yaw1: f32) -> c_int;
        pub fn roomtone_room_make(room: *mut Room);
        pub fn roomtone_room_made(room: *const Room) -> c_int;
        pub fn roomtone_room_frames(room: *const Room) -> u32;
        pub fn roomtone_room_hits(room: *const Room) -> usize;
        pub fn roomtone_room_voxels(room: *const Room) -> usize;
        pub fn roomtone_room_stale(room: *const Room) -> c_int;
        pub fn roomtone_room_seen_median(room: *const Room) -> f32;
        pub fn roomtone_room_set_points(room: *mut Room, points: *const Vec3, count: usize);
        pub fn roomtone_room_light_at_index(room: *const Room, index: usize, out: *mut Lighting) -> c_int;
        pub fn roomtone_room_spots(room: *const Room, out: *mut Spot, capacity: usize) -> usize;
        pub fn roomtone_room_summary(room: *const Room, buffer: *mut c_char, capacity: usize) -> usize;
        pub fn roomtone_replay_file(path: *const c_char, room: *mut Room) -> c_int;
        pub fn roomtone_recorder_open(path: *const c_char) -> *mut Recorder;
        pub fn roomtone_recorder_close(recorder: *mut Recorder);
        pub fn roomtone_recorder_camera(recorder: *mut Recorder, camera: *const Camera);
        pub fn roomtone_recorder_frame(recorder: *mut Recorder, frame: *const Frame);
        pub fn roomtone_recorder_hit(recorder: *mut Recorder, origin: Vec3, point: Vec3);
        pub fn roomtone_recorder_depth(recorder: *mut Recorder, image: *const DepthImage);
        pub fn roomtone_recorder_moved(recorder: *mut Recorder, p0: Vec3, yaw0: f32, p1: Vec3, yaw1: f32);
        pub fn roomtone_recorder_points(recorder: *mut Recorder, points: *const Vec3, count: usize, time_ns: i64);
        pub fn roomtone_recorder_reset(recorder: *mut Recorder);
    }
}

/// A recording being written (Roomtone's `Recorder`): a room's inputs, in
/// the order it was given them. Closed when dropped. Each frame is flushed
/// to the file as it is written (an app is often killed, not closed).
pub struct Recorder(*mut ffi::Recorder);

// The recorder owns its file and has no thread affinity.
unsafe impl Send for Recorder {}

impl Drop for Recorder {
    fn drop(&mut self) {
        unsafe { ffi::roomtone_recorder_close(self.0) }
    }
}

impl Recorder {
    /// A new recording at `path` (replacing what was there); `None` if it
    /// couldn't be made.
    pub fn create(path: &Path) -> Option<Self> {
        let path = path.to_str().and_then(|p| CString::new(p).ok())?;
        let raw = unsafe { ffi::roomtone_recorder_open(path.as_ptr()) };
        (!raw.is_null()).then_some(Self(raw))
    }
}

/// The bytes every recording starts with.
const MAGIC: &[u8; 8] = b"RTONE001";

/// Joins recordings made one after another of one room (each a
/// [`Room::continue_recording`] of the one before) into one at `out`, as if
/// a single recorder had written them all. Returns its size in bytes.
pub fn join_recordings(parts: &[impl AsRef<Path>], out: &Path) -> std::io::Result<u64> {
    let mut to = std::io::BufWriter::new(std::fs::File::create(out)?);
    to.write_all(MAGIC)?;
    let mut size = MAGIC.len() as u64;
    for part in parts {
        let mut from = std::io::BufReader::new(std::fs::File::open(part)?);
        let mut magic = [0u8; 8];
        from.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "not a Roomtone recording"));
        }
        size += std::io::copy(&mut from, &mut to)?;
    }
    to.flush()?;
    Ok(size)
}

/// A room's capture and the light made from it.
pub struct Room {
    raw: *mut ffi::Room,
    /// What a recording started now begins with (a reset keeps the camera
    /// but not the points).
    camera: Option<Camera>,
    points: Vec<Vec3>,
    /// The last frame's, for a points record.
    time_ns: i64,
    recorder: Option<Recorder>,
}

// The room owns everything it points at and has no thread affinity.
unsafe impl Send for Room {}

impl Default for Room {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Room {
    fn drop(&mut self) {
        unsafe { ffi::roomtone_room_free(self.raw) }
    }
}

impl Room {
    pub fn new() -> Self {
        Self { raw: unsafe { ffi::roomtone_room_new() }, camera: None, points: Vec::new(), time_ns: 0, recorder: None }
    }

    /// Records everything the room is given from now on into `recorder`,
    /// from a clean start: what the room had seen is dropped, and the
    /// recording begins with its camera and points, so that a replay of it
    /// (into a new room) makes exactly the light this room makes.
    /// ([`Room::make`] isn't recorded: a replay makes where the room did on
    /// its own.)
    pub fn record(&mut self, mut recorder: Recorder) {
        // (a recording already running ends here)
        self.recorder = None;
        // A new room, not a reset: a reset room isn't a new one to the last
        // bit (its light can differ from a replay's in the last places).
        unsafe {
            ffi::roomtone_room_free(self.raw);
            self.raw = ffi::roomtone_room_new();
        }
        if let Some(camera) = self.camera {
            self.set_camera(&camera);
        }
        let points = std::mem::take(&mut self.points);
        self.set_points(&points);
        if let Some(camera) = &self.camera {
            unsafe { ffi::roomtone_recorder_camera(recorder.0, camera) };
        }
        record_points(&mut recorder, &points, self.time_ns);
        self.recorder = Some(recorder);
    }

    /// Carries on recording into `next` with nothing dropped: `next` follows
    /// the recording so far, which is closed (whole) and returned. The two
    /// joined ([`join_recordings`]) are one recording.
    pub fn continue_recording(&mut self, next: Recorder) -> Option<Recorder> {
        self.recorder.replace(next)
    }

    /// Stops recording; the recording is closed when the result is dropped.
    pub fn stop_recording(&mut self) -> Option<Recorder> {
        self.recorder.take()
    }

    pub fn recording(&self) -> bool {
        self.recorder.is_some()
    }

    /// Drops everything seen (a new capture).
    pub fn reset(&mut self) {
        unsafe { ffi::roomtone_room_reset(self.raw) }
        self.points.clear();
        if let Some(r) = &self.recorder {
            unsafe { ffi::roomtone_recorder_reset(r.0) }
        }
    }

    pub fn set_camera(&mut self, camera: &Camera) {
        unsafe { ffi::roomtone_room_set_camera(self.raw, camera) }
        self.camera = Some(*camera);
        if let Some(r) = &self.recorder {
            unsafe { ffi::roomtone_recorder_camera(r.0, camera) }
        }
    }

    pub fn has_camera(&self) -> bool {
        unsafe { ffi::roomtone_room_has_camera(self.raw) != 0 }
    }

    /// Adds a frame; every few frames the light is made again.
    ///
    /// # Panics
    /// When `cells` is shorter than `cols * rows * 4`.
    pub fn add_frame(&mut self, frame: &Frame) -> FrameResult {
        assert!(frame.cells.len() >= frame.cols * frame.rows * 4, "a frame's cells are cols * rows * 4");
        let raw = ffi::Frame {
            cells: frame.cells.as_ptr(),
            cols: frame.cols as i32,
            rows: frame.rows as i32,
            exposure_ms: frame.exposure_ms,
            iso: frame.iso,
            head: frame.head,
            time_ns: frame.time_ns,
        };
        if let Some(r) = &self.recorder {
            unsafe { ffi::roomtone_recorder_frame(r.0, &raw) }
        }
        self.time_ns = frame.time_ns;
        let r = unsafe { ffi::roomtone_room_add_frame(self.raw, &raw) };
        FrameResult { used: r.used != 0, restarted: r.restarted != 0, change: r.change, black: r.black != 0 }
    }

    /// A depth hit along a ray from `origin` (a raycast): gives the lights a place.
    pub fn add_hit(&mut self, origin: Vec3, point: Vec3) {
        if let Some(r) = &self.recorder {
            unsafe { ffi::roomtone_recorder_hit(r.0, origin, point) }
        }
        unsafe { ffi::roomtone_room_add_hit(self.raw, origin, point) }
    }

    /// The latest depth image: the room's surfaces get a place (frames after
    /// it land on them), and the light is made from the points asked about.
    ///
    /// # Panics
    /// When `depth` is shorter than `stride * height`, or `stride < width`.
    pub fn add_depth(&mut self, image: &DepthImage) {
        assert!(image.stride >= image.width && image.depth.len() >= image.stride * image.height);
        let raw = ffi::DepthImage {
            depth: image.depth.as_ptr(),
            width: image.width as i32,
            height: image.height as i32,
            stride: image.stride as i32,
            near_z: image.near_z,
            far_z: image.far_z,
            tan_left: image.tan_left,
            tan_right: image.tan_right,
            tan_up: image.tan_up,
            tan_down: image.tan_down,
            view: image.view,
            time_ns: image.time_ns,
        };
        if let Some(r) = &self.recorder {
            unsafe { ffi::roomtone_recorder_depth(r.0, &raw) }
        }
        unsafe { ffi::roomtone_room_add_depth(self.raw, &raw) }
    }

    /// The tracking space moved under the room (a recenter): (p0, yaw0) to
    /// (p1, yaw1), yaw about +Y. True when the light was made again.
    pub fn moved(&mut self, p0: Vec3, yaw0: f32, p1: Vec3, yaw1: f32) -> bool {
        if let Some(r) = &self.recorder {
            unsafe { ffi::roomtone_recorder_moved(r.0, p0, yaw0, p1, yaw1) }
        }
        unsafe { ffi::roomtone_room_moved(self.raw, p0, yaw0, p1, yaw1) != 0 }
    }

    /// Makes the light from what's been seen now.
    pub fn make(&mut self) {
        unsafe { ffi::roomtone_room_make(self.raw) }
    }

    pub fn made(&self) -> bool {
        unsafe { ffi::roomtone_room_made(self.raw) != 0 }
    }

    pub fn frames(&self) -> u32 {
        unsafe { ffi::roomtone_room_frames(self.raw) }
    }

    pub fn hits(&self) -> usize {
        unsafe { ffi::roomtone_room_hits(self.raw) }
    }

    pub fn voxels(&self) -> usize {
        unsafe { ffi::roomtone_room_voxels(self.raw) }
    }

    /// Whether a make is due for a moved point.
    pub fn stale(&self) -> bool {
        unsafe { ffi::roomtone_room_stale(self.raw) != 0 }
    }

    /// The seen room's median brightness as passthrough shows it (0-1; 0 before a make).
    pub fn seen_median(&self) -> f32 {
        unsafe { ffi::roomtone_room_seen_median(self.raw) }
    }

    /// The points the light is made for (where the content is); the first
    /// sets the level's exposure. Cheap; the next frame or [`Room::make`]
    /// makes the light there.
    pub fn set_points(&mut self, points: &[Vec3]) {
        unsafe { ffi::roomtone_room_set_points(self.raw, points.as_ptr(), points.len()) }
        self.points.clear();
        self.points.extend_from_slice(points);
        if let Some(r) = &mut self.recorder {
            record_points(r, points, self.time_ns);
        }
    }

    /// The light at the `index`th of the points set (past the last: the
    /// last's); `None` while none is made.
    pub fn light_at(&self, index: usize) -> Option<Lighting> {
        let mut out = Lighting::default();
        (unsafe { ffi::roomtone_room_light_at_index(self.raw, index, &mut out) } != 0).then_some(out)
    }

    /// Every bright spot as the light was last made, brightest first.
    pub fn spots(&self) -> Vec<Spot> {
        let count = unsafe { ffi::roomtone_room_spots(self.raw, std::ptr::null_mut(), 0) };
        let mut out = vec![Spot::default(); count];
        let written = unsafe { ffi::roomtone_room_spots(self.raw, out.as_mut_ptr(), out.len()) };
        out.truncate(written);
        out
    }

    /// A line of diagnostics (coverage, lights, level, brightest cells).
    pub fn summary(&self) -> String {
        let length = unsafe { ffi::roomtone_room_summary(self.raw, std::ptr::null_mut(), 0) };
        let mut buffer = vec![0u8; length + 1];
        unsafe { ffi::roomtone_room_summary(self.raw, buffer.as_mut_ptr() as *mut c_char, buffer.len()) };
        buffer.truncate(length);
        String::from_utf8_lossy(&buffer).into_owned()
    }

    /// The points last set (none after a reset).
    pub fn points(&self) -> &[Vec3] {
        &self.points
    }

    /// Feeds a recording (Roomtone's `Recorder`, `roomtone_replay`'s input)
    /// into the room. False if it couldn't be read. (Not itself recorded,
    /// and the room keeps no camera or points of it for a recording.)
    pub fn replay(&mut self, path: &Path) -> bool {
        let Some(path) = path.to_str().and_then(|p| CString::new(p).ok()) else {
            return false;
        };
        let ok: c_int = unsafe { ffi::roomtone_replay_file(path.as_ptr(), self.raw) };
        ok != 0
    }
}

fn record_points(recorder: &mut Recorder, points: &[Vec3], time_ns: i64) {
    // (an empty one isn't recorded)
    unsafe { ffi::roomtone_recorder_points(recorder.0, points.as_ptr(), points.len(), time_ns) }
}

/// Synthetic captures, for tests and for trying the room light without a headset.
pub mod synthetic {
    use super::{Camera, Frame, Pose, Quat, Room};

    /// A Quest 3-like passthrough camera: 1280 of 1728, f 1170, looking ahead.
    pub fn camera() -> Camera {
        Camera {
            // The lens pose turns OpenCV's axes (y down, z ahead) into the
            // head's (y up, -z ahead): half a turn about X.
            lens_rotation: Quat { x: 1.0, y: 0.0, z: 0.0, w: 0.0 },
            fx: 1170.0,
            fy: 1170.0,
            cx: 864.0,
            cy: 864.0,
            sensor_width: 1728,
            sensor_height: 1728,
            width: 1280,
            height: 1280,
        }
    }

    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    }

    fn rotate(q: Quat, v: [f32; 3]) -> [f32; 3] {
        let u = [q.x, q.y, q.z];
        let t = cross(u, v).map(|c| 2.0 * c);
        let c = cross(u, t);
        [v[0] + t[0] * q.w + c[0], v[1] + t[1] * q.w + c[1], v[2] + t[2] * q.w + c[2]]
    }

    fn normalize(v: [f32; 3]) -> [f32; 3] {
        let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        v.map(|c| c / n)
    }

    /// Yaw about +Y, then pitch about the turned +X.
    fn yaw_pitch(yaw: f32, pitch: f32) -> Quat {
        let (a, b) = (
            Quat { x: 0.0, y: (yaw / 2.0).sin(), z: 0.0, w: (yaw / 2.0).cos() },
            Quat { x: (pitch / 2.0).sin(), y: 0.0, z: 0.0, w: (pitch / 2.0).cos() },
        );
        Quat {
            x: a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
            y: a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
            z: a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
            w: a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
        }
    }

    /// Sweeps a head (standing still at the origin) around a room whose
    /// radiance along a unit direction is `radiance` (linear r, g, b as the
    /// camera reads them at 10 ms, ISO 100; 1 or more clips), all the way
    /// round and from the floor to the ceiling, into `room`.
    pub fn sweep(room: &mut Room, radiance: impl Fn([f32; 3]) -> [f32; 3]) {
        sweep_with(room, radiance, |room, frame| {
            room.add_frame(frame);
        });
    }

    /// [`sweep`], each frame handed to `add` (which adds it to the room, with
    /// depth of its own, say).
    pub fn sweep_with(
        room: &mut Room,
        radiance: impl Fn([f32; 3]) -> [f32; 3],
        mut add: impl FnMut(&mut Room, &Frame),
    ) {
        const G: usize = 32;
        let camera = camera();
        room.set_camera(&camera);
        let mut cells = vec![0.0f32; G * G * 4];
        let mut time_ns = 0;
        let (yaws, pitches) = (40, 9);
        for k in 0..yaws * pitches {
            let yaw = std::f32::consts::TAU * (k % yaws) as f32 / yaws as f32;
            let pitch = -1.3 + 2.6 * (k / yaws) as f32 / (pitches - 1) as f32;
            let head = yaw_pitch(yaw, pitch);
            for gy in 0..G {
                for gx in 0..G {
                    let px = (gx as f32 + 0.5) * 1728.0 / G as f32;
                    let py = (gy as f32 + 0.5) * 1728.0 / G as f32;
                    let in_camera = [(px - camera.cx) / camera.fx, (py - camera.cy) / camera.fy, 1.0];
                    // by the lens rotation's inverse
                    let in_head = normalize(rotate(Quat { x: -1.0, y: 0.0, z: 0.0, w: 0.0 }, in_camera));
                    let rgb = radiance(normalize(rotate(head, in_head)));
                    let cell = &mut cells[(gy * G + gx) * 4..][..4];
                    for c in 0..3 {
                        cell[c] = rgb[c].min(1.0);
                    }
                    cell[3] = if rgb.iter().any(|v| *v >= 1.0) { 0.5 } else { 0.0 };
                }
            }
            let frame = Frame {
                cells: &cells,
                cols: G,
                rows: G,
                exposure_ms: 10.0,
                iso: 100.0,
                head: Pose { orientation: head, ..Default::default() },
                time_ns,
            };
            add(room, &frame);
            time_ns += 33_000_000;
        }
        room.make();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    fn lamp() -> [f32; 3] {
        let v = [0.3f32, 0.8, -0.5];
        let n = dot(v, v).sqrt();
        v.map(|c| c / n)
    }

    /// Grey walls, a little brighter up high, and a small lamp up and ahead.
    fn lamp_room() -> Room {
        let lamp = lamp();
        let mut room = Room::new();
        room.set_points(&[Vec3 { x: 0.0, y: -0.3, z: -0.7 }]);
        synthetic::sweep(&mut room, |d| {
            let v = if dot(d, lamp) > 0.12f32.cos() { 1.0 } else { 0.08 + 0.04 * d[1] };
            [v, v, v]
        });
        room
    }

    #[test]
    fn the_key_light_comes_from_the_lamp() {
        let room = lamp_room();
        assert!(room.has_camera() && room.made());
        assert_eq!(room.frames(), 360);
        let light = room.light_at(0).expect("a light was made");
        let key = light.lights[0];
        assert!(key.lit() && !key.placed(), "no depth: the key has no place");
        assert!(dot(key.dir.into(), lamp()) > 0.95, "key from {:?}", key.dir);
        assert!(light.level > 0.0 && light.mean > 0.0);
        let spots = room.spots();
        assert_eq!(spots.len(), 1, "{}", room.summary());
        assert!(dot(spots[0].dir.into(), lamp()) > 0.95);
    }

    #[test]
    fn the_soft_light_is_scaled_to_an_up_facing_surface() {
        let room = lamp_room();
        let light = room.light_at(0).unwrap();
        let mut up = light.irradiance([0.0, 1.0, 0.0]);
        for l in light.lights {
            for c in 0..3 {
                up[c] += l.color[c] * l.dir.y.max(0.0);
            }
        }
        assert!((up[1] - 1.0).abs() < 0.05, "up gets 1 with the lights, got {up:?}");
        let down = light.irradiance([0.0, -1.0, 0.0]);
        assert!(down[1] < up[1], "the room is darker below");
    }

    #[test]
    fn a_hit_near_the_lamp_gives_it_a_place() {
        let mut room = lamp_room();
        // Two hits within 10 degrees of a light place it.
        let at = lamp().map(|c| c * 2.0);
        for _ in 0..3 {
            room.add_hit(Vec3::default(), at.into());
        }
        room.make();
        assert_eq!(room.hits(), 3);
        let key = room.light_at(0).unwrap().lights[0];
        assert!(key.placed(), "{}", room.summary());
        let p: [f32; 3] = key.pos.into();
        let off = [p[0] - at[0], p[1] - at[1], p[2] - at[2]];
        assert!(dot(off, off).sqrt() < 0.05, "placed at {p:?}");
        assert!((key.range - 2.0).abs() < 0.05, "measured from the head, 2 m away: {}", key.range);
    }

    /// A window filling much of a wall (wider than a cell): it becomes the
    /// key light whole, and the soft light keeps the walls' own, so a
    /// surface facing away from it is lit as the walls are, not left with
    /// the dark side of the window's ring in the SH.
    #[test]
    fn a_wide_window_leaves_the_walls_their_light() {
        let window = |d: [f32; 3]| d[0] < -0.6 && d[1].abs() < 0.45;
        // The walls as dim as passthrough shows them beside a bright window
        // (the window clips), a little brighter toward it, from its bounce.
        let wall = |d: [f32; 3]| 0.03 - 0.01 * d[0];
        let made = |with_window: bool| {
            let mut room = Room::new();
            room.set_points(&[Vec3::default()]);
            synthetic::sweep(&mut room, |d| {
                let v = if with_window && window(d) { 1.0 } else { wall(d) };
                [v, v, v]
            });
            room.light_at(0).unwrap()
        };
        let (lit, dark) = (made(true), made(false));
        // The soft light on what faces away: the walls', as on what faces
        // along the window, against the room's without it (whose walls are
        // the same), in the SH's own units.
        let soft = |l: &Lighting, n| l.irradiance(n)[1];
        let walls = |l: &Lighting, n| soft(l, n) / soft(l, [0.0, 0.0, 1.0]);
        for n in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]] {
            let side = walls(&lit, n) / walls(&dark, n);
            assert!((0.8..1.25).contains(&side), "{n:?}: {side} of the walls' light");
        }
        // The reference (the brightness's) keeps the window's light beyond
        // its core, as it did before the light took the window whole: shown
        // against it, the walls' light is less than without the window, by
        // what the window adds to the room's mean (0.09 here, where it is 25
        // to 30 times the walls over an eighth of the sky; before the light
        // took the window whole the SH rang below that).
        let shown = |l: &Lighting, n| soft(l, n) / l.mean;
        let away = shown(&lit, [1.0, 0.0, 0.0]) / shown(&dark, [1.0, 0.0, 0.0]);
        assert!((0.06..0.12).contains(&away), "away from the window: {away} of the walls' light");
        // Toward it: the window's light on top, one light (not two halves of it).
        let key = lit.lights[0];
        assert!(key.dir.x < -0.95 && !lit.lights[1].lit(), "one light, the window: {:?}", lit.lights);
        let toward = (lit.irradiance([-1.0, 0.0, 0.0])[1] + key.color[1]) / lit.mean;
        assert!(toward > 10.0 * shown(&lit, [1.0, 0.0, 0.0]), "toward the window: {toward}");
    }

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("roomtone-sys-{name}-{}", std::process::id()))
    }

    /// Every point's light, for comparing two rooms.
    fn lights(room: &Room) -> Vec<Option<Lighting>> {
        (0..room.points().len()).map(|i| room.light_at(i)).collect()
    }

    /// A room recorded part of the way through (what it saw before is
    /// dropped), in two parts, replays into the light it made.
    #[test]
    fn a_recording_in_parts_replays_into_the_same_light() {
        let (first, second, joined) = (temp("first.rtone"), temp("second.rtone"), temp("joined.rtone"));
        let lamp = lamp();
        let mut room = Room::new();
        room.set_points(&[Vec3 { x: 0.0, y: -0.3, z: -0.7 }, Vec3 { x: 1.0, y: 0.0, z: 0.0 }]);
        let mut frames = 0;
        synthetic::sweep_with(
            &mut room,
            |d| {
                let v = if dot(d, lamp) > 0.12f32.cos() { 1.0 } else { 0.08 + 0.04 * d[1] };
                [v, v, v]
            },
            |room, frame| {
                frames += 1;
                match frames {
                    // Seen before the recording: dropped by it.
                    40 => room.record(Recorder::create(&first).unwrap()),
                    200 => {
                        drop(room.continue_recording(Recorder::create(&second).unwrap()));
                        // (in the second part)
                        room.add_hit(Vec3::default(), lamp.map(|c| c * 2.0).into());
                    }
                    _ => {}
                }
                room.add_frame(frame);
            },
        );
        assert!(room.recording());
        drop(room.stop_recording());
        let bytes = join_recordings(&[&first, &second], &joined).unwrap();
        assert_eq!(bytes, std::fs::metadata(&first).unwrap().len() + std::fs::metadata(&second).unwrap().len() - 8);
        let mut replayed = Room::new();
        assert!(replayed.replay(&joined));
        // (the sweep's last make isn't recorded)
        replayed.make();
        assert_eq!(replayed.frames(), room.frames());
        assert_eq!(replayed.hits(), 1);
        let made = lights(&room);
        assert!(made.iter().all(Option::is_some));
        let replayed_lights: Vec<Option<Lighting>> = (0..2).map(|i| replayed.light_at(i)).collect();
        assert_eq!(replayed_lights, made);
        // The first part alone is a recording too (cut where it was).
        let mut part = Room::new();
        assert!(part.replay(&first), "first part");
        assert_eq!((part.frames(), part.hits()), (160, 0));
        for p in [first, second, joined] {
            let _ = std::fs::remove_file(p);
        }
    }

    /// A room recorded from its start (as the app's is when the developer
    /// setting is on at launch: no camera or points yet) replays too.
    #[test]
    fn a_room_recorded_from_nothing_replays_into_the_same_light() {
        let path = temp("fresh.rtone");
        let mut room = Room::new();
        room.record(Recorder::create(&path).unwrap());
        room.set_points(&[Vec3 { x: 0.0, y: -0.3, z: -0.7 }]);
        synthetic::sweep(&mut room, |d| [0.1 + 0.05 * d[1]; 3]);
        drop(room.stop_recording());
        let mut replayed = Room::new();
        assert!(replayed.replay(&path));
        replayed.make();
        assert!(room.light_at(0).is_some());
        assert_eq!(replayed.light_at(0), room.light_at(0));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_missing_recording_does_not_replay() {
        assert!(!Room::new().replay(Path::new("no-such-file.roomtone")));
    }

    #[test]
    fn summary_and_empty_room() {
        let room = Room::new();
        assert!(!room.made() && room.light_at(0).is_none() && room.spots().is_empty());
        assert!(!lamp_room().summary().is_empty());
    }

    #[test]
    fn a_black_image_is_left_out() {
        let mut room = lamp_room();
        let level = room.light_at(0).unwrap().level;
        let cells = vec![0.0005f32; 32 * 32 * 4];
        let frame = Frame { cells: &cells, cols: 32, rows: 32, exposure_ms: 33.0, iso: 3200.0, head: Pose::default(), time_ns: 0 };
        for i in 0..20 {
            let r = room.add_frame(&Frame { time_ns: 100_000_000_000 + i * 200_000_000, ..frame });
            assert!(r.black && !r.used && !r.restarted, "{r:?}");
        }
        assert_eq!(room.light_at(0).unwrap().level, level);
    }
}
