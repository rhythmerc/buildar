// The room's depth image with the player's hands and arms moved on in
// time. The image is some 80 ms old when it is shown, and a fast hand is
// far from where it was; but the controllers are tracked with no such
// wait. So what is about each controller's palm in the image (the hand)
// is taken to have gone where the palm went since, and what is along the
// line from the palm to the shoulder (the arm) to have swung about the
// shoulder as the palm went.
//
// Two draws into a depth image of our own. `base`: each texel as the image
// had it, but where what was there has moved off, what stays put nearest
// about it and behind it. Then `mesh`: the image as a sheet of small
// squares (one to every two texels each way), each corner moved as its
// point of the room moved, drawn over the first where it is nearer. A
// square across an edge in depth is left out, so nothing is stretched
// from a hand to what is behind it; and how much a point moves fades over
// a hand's width past a part's edge, so nothing is torn off a part either.
struct Part {
    // How the part's points moved, and back.
    forward: mat4x4<f32>,
    backward: mat4x4<f32>,
    // The part's line where it is now (for a first guess of its distance).
    line_start: vec4<f32>,
    line_end: vec4<f32>,
    // The hand's middle when the image was taken, and its reach.
    centre: vec4<f32>,
    // An arm: the wrist then and the arm's length; the way to the shoulder and the arm's thickness.
    // (Thickness 0: a hand, whose `wrist` is the controller's top and how far about it is the hand's too.)
    wrist: vec4<f32>,
    axis: vec4<f32>,
}

struct Moves {
    view_from_tracking: mat4x4<f32>,
    tracking_from_view: mat4x4<f32>,
    tangents: vec4<f32>,
    near: f32,
    far: f32,
    count: u32,
    parts: array<Part, 4>,
}

@group(0) @binding(0) var room_depth: texture_depth_2d;
@group(0) @binding(1) var<uniform> moves: Moves;

fn metres(d: f32) -> f32 {
    if d <= 0.0 || d >= 1.0 {
        return 0.0;
    }
    if moves.far <= moves.near || moves.far > 1e6 {
        return moves.near / (1.0 - d);
    }
    return moves.near * moves.far / (moves.far - d * (moves.far - moves.near));
}

// Metres as the image's own value.
fn depth_of(z: f32) -> f32 {
    if z <= moves.near {
        return 0.0;
    }
    if moves.far <= moves.near || moves.far > 1e6 {
        return 1.0 - moves.near / z;
    }
    return moves.far * (z - moves.near) / (z * (moves.far - moves.near));
}

fn point(image: vec2<f32>, z: f32) -> vec3<f32> {
    let t = moves.tangents;
    return (moves.tracking_from_view * vec4((t.x + image.x * (t.y - t.x)) * z, (t.z + image.y * (t.w - t.z)) * z, -z, 1.0)).xyz;
}

// A point's place in the image and its metres ahead.
fn place(p: vec3<f32>) -> vec3<f32> {
    let v = (moves.view_from_tracking * vec4(p, 1.0)).xyz;
    let ahead = max(-v.z, 1e-4);
    let t = moves.tangents;
    return vec3((v.x / ahead - t.x) / (t.y - t.x), (v.y / ahead - t.z) / (t.w - t.z), -v.z);
}

fn metres_at(image: vec2<f32>, size: vec2<i32>) -> f32 {
    return metres(textureLoad(room_depth, clamp(vec2<i32>(floor(image * vec2<f32>(size))), vec2(0), size - 1), 0));
}

// How much a point is a part's (1: all of it; 0: not at all). It fades
// over a hand's width past the part's edge, so that what is beside a part
// is carried a little with it and nothing is torn off it.
const FADE_M: f32 = 0.08;

fn hand_share(part: Part, p: vec3<f32>) -> f32 {
    return 1.0 - smoothstep(0.0, FADE_M, distance(p, part.centre.xyz) - part.centre.w);
}

fn share(part: Part, p: vec3<f32>) -> f32 {
    if part.axis.w <= 0.0 {
        // (a hand: about the palm, or about the controller's top, which stands out of it)
        return max(hand_share(part, p), 1.0 - smoothstep(0.0, FADE_M, distance(p, part.wrist.xyz) - part.wrist.w));
    }
    let from_wrist = p - part.wrist.xyz;
    let along = dot(from_wrist, part.axis.xyz);
    let off = length(from_wrist - along * part.axis.xyz);
    let on_line = smoothstep(-FADE_M, 0.0, along) * (1.0 - smoothstep(part.wrist.w, part.wrist.w + FADE_M, along));
    // (what is the hand's is not also the arm's)
    return (1.0 - smoothstep(0.0, FADE_M, off - part.axis.w)) * on_line * (1.0 - hand_share(part, p));
}

// How far a point of the room moved: each part's motion, by how much the point is the part's.
fn moved(p: vec3<f32>) -> vec3<f32> {
    var by = vec3(0.0);
    for (var i = 0u; i < moves.count; i++) {
        let part = moves.parts[i];
        by += share(part, p) * ((part.forward * vec4(p, 1.0)).xyz - p);
    }
    return by;
}

// How far a point's move takes it across the image, in texels.
fn slides(p: vec3<f32>, image: vec2<f32>, size: vec2<i32>) -> f32 {
    return length((place(p + moved(p)).xy - image) * vec2<f32>(size));
}

@fragment
fn base(@builtin(position) position: vec4<f32>) -> @builtin(frag_depth) f32 {
    let size = vec2<i32>(textureDimensions(room_depth));
    let image = position.xy / vec2<f32>(size);
    let raw = textureLoad(room_depth, vec2<i32>(position.xy), 0);
    let own = metres(raw);
    if moves.count == 0u || own <= 0.0 || slides(point(image, own), image, size) < 1.0 {
        return raw;
    }
    // What was here has moved off: what was just behind it shows, by what
    // stays put nearest about here. The nearest ring of texels with any
    // that are behind where the thing was; of those, the nearest behind.
    // (The farthest instead made holes in real things a hand passed in
    // front of.)
    var behind = 0.0;
    for (var reach = 2; reach <= 32 && behind <= 0.0; reach *= 2) {
        for (var way = 0; way < 8; way++) {
            let dir = array<vec2<f32>, 8>(vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(-1.0, -1.0))[way];
            let there = image + dir * f32(reach) / vec2<f32>(size);
            let m = metres_at(there, size);
            if m > own * 0.97 && (behind <= 0.0 || m < behind) && slides(point(there, m), there, size) < 1.0 {
                behind = m;
            }
        }
    }
    // (nothing found: as the image had it)
    return select(raw, depth_of(behind), behind > 0.0);
}

// The sheet's squares: this many each way, two texels to a side.
const SQUARES: u32 = 160u;

@vertex
fn mesh(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let size = vec2<i32>(textureDimensions(room_depth));
    let square = index / 6u;
    let corner = array<vec2<u32>, 6>(vec2(0u, 0u), vec2(1u, 0u), vec2(0u, 1u), vec2(0u, 1u), vec2(1u, 0u), vec2(1u, 1u))[index % 6u];
    let cell = vec2(square % SQUARES, square / SQUARES);
    let off = vec4(0.0, 0.0, 2.0, 1.0);
    // The square's corners: an edge in depth across it, or nothing known, or none of it moving, and it is left out.
    var nearest = 1e9;
    var farthest = 0.0;
    var moving = false;
    var mine = vec3(0.0);
    for (var c = 0u; c < 4u; c++) {
        let at = min(vec2<i32>((cell + vec2(c % 2u, c / 2u)) * 2u), size - 1);
        let z = metres(textureLoad(room_depth, at, 0));
        nearest = min(nearest, z);
        farthest = max(farthest, z);
        let image = (vec2<f32>(at) + 0.5) / vec2<f32>(size);
        let p = point(image, z);
        let to = p + moved(p);
        moving = moving || length((place(to).xy - image) * vec2<f32>(size)) >= 1.0;
        if c == corner.x + corner.y * 2u {
            mine = to;
        }
    }
    if moves.count == 0u || nearest <= 0.0 || farthest > nearest * 1.1 || !moving {
        return off;
    }
    let lands = place(mine);
    if lands.z <= moves.near {
        return off;
    }
    // (the image's rows run from its first, which is the top of the target)
    return vec4(lands.x * 2.0 - 1.0, 1.0 - lands.y * 2.0, depth_of(lands.z), 1.0);
}
