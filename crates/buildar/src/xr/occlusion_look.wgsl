// What the passes that find occlusion's edges in the camera's image share.
#define_import_path buildar::xr::occlusion_look

struct Look {
    // The depth image's view to the room's tracking space, and that to the camera's (x right, y down, z ahead).
    tracking_from_view: mat4x4<f32>,
    camera_from_tracking: mat4x4<f32>,
    // The view the kept answers were worked out in (the frame before's).
    old_view_from_tracking: mat4x4<f32>,
    // The image's left, right, down and up edges, as tangents; and the old view's.
    tangents: vec4<f32>,
    old_tangents: vec4<f32>,
    // The camera's (fx, fy, cx, cy), over its frame's size.
    intrinsics: vec4<f32>,
    near: f32,
    far: f32,
    // 1: there is a camera frame. 2: there are kept answers.
    flags: u32,
    // How much of the kept evidence stays each frame, and the most there can be.
    keep: f32,
    most: f32,
}

@group(1) @binding(0) var<uniform> look: Look;

// A value of the room's image as metres along its view (0: unknown).
fn metres(d: f32) -> f32 {
    if d <= 0.0 || d >= 1.0 {
        return 0.0;
    }
    if look.far <= look.near || look.far > 1e6 {
        return look.near / (1.0 - d);
    }
    return look.near * look.far / (look.far - d * (look.far - look.near));
}

// The point of the room at a place in the depth image (0-1, rows from the bottom) and a distance.
fn room_point(image: vec2<f32>, z: f32) -> vec3<f32> {
    let t = look.tangents;
    let view = vec3((t.x + image.x * (t.y - t.x)) * z, (t.z + image.y * (t.w - t.z)) * z, -z);
    return (look.tracking_from_view * vec4(view, 1.0)).xyz;
}

// Where a point of the room is in the camera's frame (0-1, rows from the top); z: 1 when it is in it.
fn in_camera(point: vec3<f32>) -> vec3<f32> {
    let p = (look.camera_from_tracking * vec4(point, 1.0)).xyz;
    if (look.flags & 1u) == 0u || p.z <= 0.05 {
        return vec3(0.0);
    }
    let uv = look.intrinsics.xy * p.xy / p.z + look.intrinsics.zw;
    return vec3(uv, f32(all(uv > vec2(0.002)) && all(uv < vec2(0.998))));
}
