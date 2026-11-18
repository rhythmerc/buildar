//! Occlusion's edges found in the camera's image (the AR settings' "sharper
//! edges", under occlusion): what this needs from outside the renderer.
//!
//! The depth image is right about surfaces and wrong about where they end,
//! by a texel or three (a texel is some seven pixels of the picture), and
//! its edges wander. The left passthrough camera sees the real edges: where
//! a patch of the depth image has near things and far things, each pixel
//! is put with whichever its colour in the camera's frame is like, and that
//! answer is kept from frame to frame so it takes steady evidence to turn
//! a pixel over (`occlusion_matte.wgsl`).
//!
//! The camera's thread (room lighting's: `room_light::camera`) leaves its
//! newest frame here at half size, with the camera's calibration; the game
//! adds where the camera was when it was taken, from the head's poses it
//! has kept by time, and the renderer takes it from there. Frames are only
//! ever looked at on the headset, and none is stored.
use bevy::prelude::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The game wants camera frames for edges (the setting is on, and allowed).
static WANTED: AtomicBool = AtomicBool::new(false);
static LATEST: Mutex<Option<GuideFrame>> = Mutex::new(None);

/// A frame older than this is no guide (the camera stopped, or was lent out).
const STALE_NS: i64 = 250_000_000;
/// How many frames' head poses are kept (over half a second of them).
#[cfg(not(test))]
const POSES: usize = 48;
/// (a whole capture's worth, for replaying one)
#[cfg(test)]
const POSES: usize = 4096;

/// A camera frame for the renderer: half the stream's size, RGBA.
#[derive(Clone)]
pub(crate) struct GuideFrame {
    pub pixels: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    /// When it was taken, on the steady clock (which is OpenXR's time here).
    pub timestamp_ns: i64,
    /// Camera2's lens pose: turns the head's axes into the camera's (x right,
    /// y down, z ahead), and the lens in the head's space.
    pub lens_rotation: Quat,
    pub lens_translation: Vec3,
    /// (fx, fy, cx, cy) over the sensor's size: a point `p` in the camera's
    /// space is at `fx * p.x / p.z + cx` across the frame, 0-1.
    pub intrinsics: Vec4,
}

#[cfg(target_os = "android")]
pub(crate) fn wanted() -> bool {
    WANTED.load(Ordering::Relaxed)
}

pub(crate) fn set_wanted(wanted: bool) {
    WANTED.store(wanted, Ordering::Relaxed);
    if !wanted {
        *LATEST.lock().unwrap() = None;
    }
}

/// The camera's thread: its newest frame.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn publish(frame: GuideFrame) {
    *LATEST.lock().unwrap() = Some(frame);
}

fn latest() -> Option<GuideFrame> {
    LATEST.lock().unwrap().clone()
}

/// The head (the middle of the eyes) in tracking space at each of the last
/// frames' display times.
#[derive(Resource, Default, Clone)]
pub(crate) struct HeadPoses(VecDeque<(i64, Vec3, Quat)>);

/// The controllers' aim poses, left and right, the same way.
#[derive(Resource, Default, Clone)]
pub(crate) struct HandPoses(pub [HeadPoses; 2]);

/// The palm from the controller's aim pose (in that pose's axes), m, and
/// how far about it the hand is taken to reach. The hand turns with the
/// controller about the palm; a wider reach, or a middle further from the
/// palm, took in the wrist and what the hand was near, and a twist of the
/// controller swung them about with it.
const PALM_M: Vec3 = Vec3::new(0.0, -0.03, 0.06);
const HAND_REACH_M: f32 = 0.09;
/// The controller's top stands out of the hand: what is this near the aim
/// pose itself is the hand's too. (Left out, it stayed behind when the
/// hand went, and the two came apart.)
const TOP_REACH_M: f32 = 0.07;
/// How thick an arm is about its line, m.
const ARM_M: f32 = 0.07;
/// Where a shoulder is from the head: out to its side, down, and back, m.
const SHOULDER_M: Vec3 = Vec3::new(0.19, 0.22, 0.02);

/// A part of the player that moved between two times, for
/// `occlusion_advance.wgsl`: how its points moved, its line now, and what
/// of the room was it then.
#[derive(Clone, Copy, Debug, Default, bevy::render::render_resource::ShaderType)]
pub(crate) struct Part {
    pub forward: Mat4,
    pub backward: Mat4,
    pub line_start: Vec4,
    pub line_end: Vec4,
    /// The hand's middle then, and its reach.
    pub centre: Vec4,
    /// An arm: the wrist then and the arm's length; the way to the shoulder
    /// and the arm's thickness (0: a hand, whose `wrist` is then the
    /// controller's top and how far about it is the hand's too).
    pub wrist: Vec4,
    pub axis: Vec4,
}

/// The hands and arms as they moved from `then` to `now` (ns): each hand
/// rigidly with its controller, each arm swung about its shoulder (which
/// is taken to be under the head) as the wrist went.
pub(crate) fn parts(head: &HeadPoses, hands: &HandPoses, then: i64, now: i64) -> Vec<Part> {
    let mut out = Vec::new();
    let Some((head_at, head_turn)) = head.at(then) else { return out };
    let ahead = (head_turn * Vec3::NEG_Z).with_y(0.0).normalize_or_zero();
    let right = ahead.cross(Vec3::Y);
    for (side, poses) in [(-1.0, &hands.0[0]), (1.0, &hands.0[1])] {
        let (Some((pa, qa)), Some((pb, qb))) = (poses.at(then), poses.at(now)) else { continue };
        let centre = pa + qa * PALM_M;
        let centre_now = (pb + qb * PALM_M).extend(0.0);
        // The hand goes where the palm went, and is not turned with the
        // controller: the depth image has only the side of the hand that
        // faced the headset, and a quick twist (60 degrees in 50 ms is
        // usual) turned that shell to show a back it doesn't have, which
        // broke the hand up.
        let forward = Mat4::from_translation(centre_now.truncate() - centre);
        out.push(Part {
            forward,
            backward: forward.inverse(),
            line_start: centre_now,
            line_end: centre_now,
            centre: centre.extend(HAND_REACH_M),
            // (a hand's `wrist` is its controller's top, and how far about it)
            wrist: pa.extend(TOP_REACH_M),
            axis: Vec4::ZERO,
        });
        let shoulder = head_at + right * SHOULDER_M.x * side - Vec3::Y * SHOULDER_M.y - ahead * SHOULDER_M.z;
        // (the arm's line is from the palm: a twist of the controller about it doesn't swing the arm)
        let (wrist, wrist_now) = (centre, centre_now.truncate());
        let (arm, arm_now) = (wrist - shoulder, wrist_now - shoulder);
        let length = arm.length();
        if length < 0.05 || arm_now.length() < 0.05 {
            continue;
        }
        let swing = Mat4::from_translation(shoulder)
            * Mat4::from_scale(Vec3::splat(arm_now.length() / length))
            * Mat4::from_quat(Quat::from_rotation_arc(arm / length, arm_now.normalize()))
            * Mat4::from_translation(-shoulder);
        out.push(Part {
            forward: swing,
            backward: swing.inverse(),
            line_start: wrist_now.extend(0.0),
            line_end: shoulder.extend(0.0),
            centre: centre.extend(HAND_REACH_M),
            wrist: wrist.extend(length),
            axis: (-arm / length).extend(ARM_M),
        });
    }
    out
}

impl HeadPoses {
    fn add(&mut self, time_ns: i64, position: Vec3, rotation: Quat) {
        if self.0.back().is_some_and(|&(last, ..)| time_ns <= last) {
            return;
        }
        if self.0.len() == POSES {
            self.0.pop_front();
        }
        self.0.push_back((time_ns, position, rotation));
    }

    #[cfg(test)]
    pub(crate) fn add_for_test(&mut self, time_ns: i64, position: Vec3, rotation: Quat) {
        let keep = self.0.len() + 1;
        self.add(time_ns, position, rotation);
        // (a whole capture's worth, for replaying one)
        let _ = keep;
    }

    /// The head at a time, between the two poses about it (the nearest one
    /// when the time is outside what is kept).
    pub(crate) fn at(&self, time_ns: i64) -> Option<(Vec3, Quat)> {
        let after = self.0.iter().position(|&(t, ..)| t >= time_ns);
        match after {
            None => self.0.back().map(|&(_, p, q)| (p, q)),
            Some(0) => self.0.front().map(|&(_, p, q)| (p, q)),
            Some(i) => {
                let ((t0, p0, q0), (t1, p1, q1)) = (self.0[i - 1], self.0[i]);
                let share = (time_ns - t0) as f32 / (t1 - t0).max(1) as f32;
                Some((p0.lerp(p1, share), q0.slerp(q1, share)))
            }
        }
    }
}

/// The eyes' poses as each frame is drawn.
pub(super) fn keep_head_poses(
    frame: Option<Res<bevy_mod_openxr::resources::OxrFrameState>>,
    // (The eye cameras' own transforms are tracking space.)
    eyes: Query<&Transform, With<bevy_mod_xr::camera::XrCamera>>,
    hands: Query<(&Transform, &super::actions::Hand), Without<bevy_mod_xr::camera::XrCamera>>,
    mut poses: ResMut<HeadPoses>,
    mut hand_poses: ResMut<HandPoses>,
) {
    let (Some(frame), Some(first)) = (frame, eyes.iter().next()) else { return };
    for (at, hand) in &hands {
        let side = if matches!(hand, super::actions::Hand::Left) { 0 } else { 1 };
        hand_poses.0[side].add(frame.predicted_display_time.as_nanos(), at.translation, at.rotation);
    }
    let count = eyes.iter().count() as f32;
    let middle = eyes.iter().map(|t| t.translation).sum::<Vec3>() / count;
    poses.add(frame.predicted_display_time.as_nanos(), middle, first.rotation);
}

/// The camera's space from tracking space, for a camera with this lens pose
/// on a head at this pose.
pub(crate) fn camera_from_tracking(head: (Vec3, Quat), lens_rotation: Quat, lens_translation: Vec3) -> Mat4 {
    Mat4::from_quat(lens_rotation) * Mat4::from_translation(-lens_translation) * Mat4::from_rotation_translation(head.1, head.0).inverse()
}

/// What the renderer needs of the newest camera frame.
#[derive(Clone)]
pub(crate) struct Guide {
    pub frame: GuideFrame,
    pub camera_from_tracking: Mat4,
}

/// The newest frame with where its camera was; none while there is no
/// fresh one (`now_ns`: the frame being drawn's time).
pub(crate) fn guide(poses: &HeadPoses, now_ns: i64) -> Option<Guide> {
    let frame = latest()?;
    if now_ns - frame.timestamp_ns > STALE_NS {
        return None;
    }
    let head = poses.at(frame.timestamp_ns)?;
    Some(Guide { camera_from_tracking: camera_from_tracking(head, frame.lens_rotation, frame.lens_translation), frame })
}

/// A camera frame's planes (as Camera2's YUV_420_888 gives them) at half
/// size, RGBA: each pixel the mean of four of the frame's.
///
/// Row by row over slices, in fixed point (colour weights in 256ths): each
/// sample read through a bounds-checked closure in floats, this was a third
/// of a core on the headset. Rows or samples a plane is short of read as 0.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn half_rgba(
    y: (&[u8], usize, usize),
    u: (&[u8], usize, usize),
    v: (&[u8], usize, usize),
    width: usize,
    height: usize,
) -> Vec<u8> {
    let (w, h) = (width / 2, height / 2);
    let mut out = vec![255u8; w * h * 4];
    fn row<'a>((data, stride, _): (&'a [u8], usize, usize), at: usize, len: usize) -> &'a [u8] {
        let from = (at * stride).min(data.len());
        &data[from..(from + len).min(data.len())]
    }
    let get = |r: &[u8], i: usize| r.get(i).copied().unwrap_or(0) as i32;
    for py in 0..h {
        let (y0, y1) = (row(y, py * 2, w * 2 * y.2.max(1)), row(y, py * 2 + 1, w * 2 * y.2.max(1)));
        let (ur, vr) = (row(u, py, w * u.2.max(1)), row(v, py, w * v.2.max(1)));
        let (ys, us, vs) = (y.2.max(1), u.2.max(1), v.2.max(1));
        let line = &mut out[py * w * 4..(py + 1) * w * 4];
        let full = y0.len() >= w * 2 * ys && y1.len() >= w * 2 * ys && ur.len() >= (w - 1) * us + 1 && vr.len() >= (w - 1) * vs + 1;
        for (px, o) in line.chunks_exact_mut(4).enumerate() {
            let (a, b) = (px * 2 * ys, (px * 2 + 1) * ys);
            // (luma x 4 x 64 = luma in 256ths)
            let (l, cb, cr) = if full {
                (
                    (y0[a] as i32 + y0[b] as i32 + y1[a] as i32 + y1[b] as i32) * 64,
                    ur[px * us] as i32 - 128,
                    vr[px * vs] as i32 - 128,
                )
            } else {
                ((get(y0, a) + get(y0, b) + get(y1, a) + get(y1, b)) * 64, get(ur, px * us) - 128, get(vr, px * vs) - 128)
            };
            // (1.402, 0.344, 0.714 and 1.772 in 256ths, rounded)
            o[0] = ((l + 359 * cr + 128) >> 8).clamp(0, 255) as u8;
            o[1] = ((l - 88 * cb - 183 * cr + 128) >> 8).clamp(0, 255) as u8;
            o[2] = ((l + 454 * cb + 128) >> 8).clamp(0, 255) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_head_at_a_time_is_between_the_poses_about_it() {
        let mut poses = HeadPoses::default();
        assert!(poses.at(5).is_none());
        poses.add(100, Vec3::ZERO, Quat::IDENTITY);
        poses.add(200, Vec3::X, Quat::from_rotation_y(1.0));
        // (a time that goes back is left out)
        poses.add(150, Vec3::splat(9.0), Quat::IDENTITY);
        let (p, q) = poses.at(150).unwrap();
        assert!((p - Vec3::X * 0.5).length() < 1e-5);
        assert!(q.angle_between(Quat::from_rotation_y(0.5)) < 1e-4);
        assert_eq!(poses.at(50).unwrap().0, Vec3::ZERO, "before them all: the first");
        assert_eq!(poses.at(900).unwrap().0, Vec3::X, "after them all: the last");
        for i in 0..100 {
            poses.add(1000 + i, Vec3::ZERO, Quat::IDENTITY);
        }
        assert!(poses.0.len() <= POSES);
    }

    /// A point of a recorded session lands where the desk-side working of
    /// the same session (numpy, checked by eye against the camera's frames)
    /// puts it.
    #[test]
    fn a_point_in_the_room_lands_in_the_cameras_frame_where_the_recording_says() {
        let head = (
            Vec3::new(0.6093691415335974, 1.0596616018789826, 0.22870923971612417),
            Quat::from_xyzw(0.04174255014991297, -0.9713376535451258, -0.17832329636807462, 0.1515306051095127),
        );
        let lens_rotation = Quat::from_xyzw(-0.9952278733253479, -0.00039808754809200764, -0.004327708855271339, 0.09748116880655289);
        let lens_translation = Vec3::new(-0.03182709962129593, -0.01681988500058651, -0.06260181218385696);
        let intrinsics = Vec4::new(1169.6590576171875, 1169.6590576171875, 864.3427124023438, 863.6021118164062) / 1728.0;
        let p = camera_from_tracking(head, lens_rotation, lens_translation).transform_point3(Vec3::new(0.9, 0.7, 1.6));
        assert!((p.z - 1.31182652).abs() < 1e-3, "{p}");
        let uv = Vec2::new(intrinsics.x * p.x / p.z + intrinsics.z, intrinsics.y * p.y / p.z + intrinsics.w);
        assert!((uv - Vec2::new(0.591012, 0.27882281)).length() < 1e-3, "{uv}");
    }

    #[test]
    fn a_hand_moves_with_its_controller_and_its_arm_swings_about_the_shoulder() {
        let mut head = HeadPoses::default();
        head.add(0, Vec3::new(0.0, 1.2, 0.0), Quat::IDENTITY);
        let mut hands = HandPoses::default();
        // The right controller, ahead and to the right, goes 20 cm to the left and turns.
        let (pa, pb) = (Vec3::new(0.3, 0.9, -0.4), Vec3::new(0.1, 0.9, -0.4));
        hands.0[1].add(0, pa, Quat::IDENTITY);
        hands.0[1].add(100, pb, Quat::from_rotation_y(0.5));
        let parts = parts(&head, &hands, 0, 100);
        assert_eq!(parts.len(), 2, "its hand and its arm (no left controller: no left parts)");
        let (hand, arm) = (parts[0], parts[1]);
        // The hand goes as the palm went, unturned.
        let palm_went = pb + Quat::from_rotation_y(0.5) * PALM_M - (pa + PALM_M);
        assert!((hand.forward.transform_point3(pa) - (pa + palm_went)).length() < 1e-5);
        assert!((hand.backward.transform_point3(pa + palm_went) - pa).length() < 1e-5);
        assert!((hand.forward.transform_vector3(Vec3::X) - Vec3::X).length() < 1e-6);
        assert_eq!(hand.axis.w, 0.0);
        // The wrist goes with the controller; the shoulder stays; the arm's middle goes about half as far.
        let shoulder = Vec3::new(0.19, 0.98, 0.02);
        let (wrist, wrist_now) = (pa + PALM_M, pb + Quat::from_rotation_y(0.5) * PALM_M);
        assert!((arm.forward.transform_point3(wrist) - wrist_now).length() < 1e-4);
        assert!((arm.forward.transform_point3(shoulder) - shoulder).length() < 1e-5);
        let middle = (wrist + shoulder) * 0.5;
        let moved = (arm.forward.transform_point3(middle) - middle).length();
        assert!((moved / (wrist_now - wrist).length() - 0.5).abs() < 0.1, "{moved}");
        // The arm's line runs from the wrist to the shoulder, and is its length.
        assert!((arm.wrist.xyz() + arm.axis.xyz() * arm.wrist.w - shoulder).length() < 1e-4);
        assert!((arm.line_end.xyz() - shoulder).length() < 1e-5);
        // A twist of the controller about the palm leaves the arm where it was.
        let mut twisted = HandPoses::default();
        let twist = Quat::from_rotation_z(1.0);
        twisted.0[1].add(0, pa, Quat::IDENTITY);
        twisted.0[1].add(100, pa + PALM_M - twist * PALM_M, twist);
        let arm = super::parts(&head, &twisted, 0, 100)[1];
        assert!((arm.forward.transform_point3(middle) - middle).length() < 1e-4);
        // Nothing moved: nothing to do but the same.
        let still = super::parts(&head, &hands, 100, 100);
        assert!((still[0].forward.transform_point3(Vec3::ONE) - Vec3::ONE).length() < 1e-5);
    }

    #[test]
    fn a_frame_at_half_size_is_the_mean_of_fours_in_colour() {
        // 4x2 of grey 100 and 200 (left and right halves), no colour.
        let y = [100, 100, 200, 200, 100, 100, 200, 200];
        let (u, v) = ([128, 0, 128, 0], [128, 0, 128, 0]);
        // (chroma with a pixel stride of 2, as the headset gives it)
        let out = half_rgba((&y, 4, 1), (&u, 4, 2), (&v, 4, 2), 4, 2);
        assert_eq!(out, [100, 100, 100, 255, 200, 200, 200, 255]);
        // Red-difference raises red and lowers green.
        let out = half_rgba((&y, 4, 1), (&u, 4, 2), (&[228, 0, 128, 0], 4, 2), 4, 2);
        assert!(out[0] > 200 && out[1] < 60 && out[2] == 100, "{out:?}");
    }

    /// The fixed-point conversion is the float one's to within 1, strides and
    /// all (a pseudo-random frame, chroma interleaved as the headset gives it).
    #[test]
    fn a_frame_at_half_size_matches_the_float_conversion() {
        let (width, height) = (64, 48);
        let mut seed = 12345u32;
        let mut next = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 24) as u8
        };
        let y: Vec<u8> = (0..72 * height).map(|_| next()).collect();
        let uv: Vec<u8> = (0..72 * height / 2).map(|_| next()).collect();
        let (u, v) = ((&uv[..], 72, 2), (&uv[1..], 72, 2));
        let out = half_rgba((&y, 72, 1), u, v, width, height);
        let at = |(data, row, pixel): (&[u8], usize, usize), x: usize, yy: usize| data.get(yy * row + x * pixel).copied().unwrap_or(0) as f32;
        for py in 0..height / 2 {
            for px in 0..width / 2 {
                let luma = (at((&y, 72, 1), px * 2, py * 2) + at((&y, 72, 1), px * 2 + 1, py * 2) + at((&y, 72, 1), px * 2, py * 2 + 1) + at((&y, 72, 1), px * 2 + 1, py * 2 + 1)) * 0.25;
                let (cb, cr) = (at(u, px, py) - 128.0, at(v, px, py) - 128.0);
                let want = [luma + 1.402 * cr, luma - 0.344 * cb - 0.714 * cr, luma + 1.772 * cb].map(|c| c.clamp(0.0, 255.0));
                let o = (py * width / 2 + px) * 4;
                for c in 0..3 {
                    assert!((out[o + c] as f32 - want[c]).abs() <= 1.5, "pixel {px},{py} channel {c}: {} vs {}", out[o + c], want[c]);
                }
            }
        }
    }

    #[test]
    fn a_stale_frame_is_no_guide() {
        let mut poses = HeadPoses::default();
        poses.add(1_000_000_000, Vec3::ZERO, Quat::IDENTITY);
        publish(GuideFrame {
            pixels: Arc::new(vec![0; 4]),
            width: 1,
            height: 1,
            timestamp_ns: 1_000_000_000,
            lens_rotation: Quat::IDENTITY,
            lens_translation: Vec3::ZERO,
            intrinsics: Vec4::ONE,
        });
        assert!(guide(&poses, 1_050_000_000).is_some());
        assert!(guide(&poses, 1_400_000_000).is_none());
        set_wanted(false);
        assert!(guide(&poses, 1_050_000_000).is_none(), "turned off: the frame is let go");
    }
}
