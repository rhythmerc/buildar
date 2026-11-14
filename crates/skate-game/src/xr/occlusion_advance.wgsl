// The room's depth image with the player's hands and arms moved on in
// time. The image is some 80 ms old when it is shown, and a fast hand is
// far from where it was; but the controllers are tracked with no such
// wait. So what is about each controller in the image (the hand) is taken
// to have moved as the controller did since, and what is along the line
// from the wrist to the shoulder (the arm) to have swung about the
// shoulder as the wrist went. For each texel and each such part, the piece
// of the part's surface that lands on the texel is looked for; where a
// part has left and nothing came, the farthest of what stays put about it
// shows through.
struct Part {
    // How the part's points moved, and back.
    forward: mat4x4<f32>,
    backward: mat4x4<f32>,
    // The part's line where it is now (for a first guess of its distance).
    line_start: vec4<f32>,
    line_end: vec4<f32>,
    // The hand's middle when the image was taken, and its reach.
    centre: vec4<f32>,
    // An arm: the wrist then and the arm's length; the way to the shoulder and the arm's thickness. (w 0: a hand.)
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

fn belongs(part: Part, p: vec3<f32>) -> bool {
    let in_hand = distance(p, part.centre.xyz) < part.centre.w;
    if part.axis.w <= 0.0 {
        return in_hand;
    }
    let from_wrist = p - part.wrist.xyz;
    let along = dot(from_wrist, part.axis.xyz);
    let off = length(from_wrist - along * part.axis.xyz);
    return along > -0.02 && along < part.wrist.w + 0.05 && off < part.axis.w && !in_hand;
}

fn moves_at(p: vec3<f32>) -> bool {
    var any = false;
    for (var i = 0u; i < moves.count; i++) {
        any = any || belongs(moves.parts[i], p);
    }
    return any;
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @builtin(frag_depth) f32 {
    let size = vec2<i32>(textureDimensions(room_depth));
    let image = position.xy / vec2<f32>(size);
    let raw = textureLoad(room_depth, vec2<i32>(position.xy), 0);
    let own = metres(raw);
    if moves.count == 0u {
        return raw;
    }
    let none = 1e9;
    // What stays put stays; what moves has left.
    var best = none;
    if own > 0.0 && !moves_at(point(image, own)) {
        best = own;
    }
    for (var i = 0u; i < moves.count; i++) {
        let part = moves.parts[i];
        // First guess: as far as the part's line is where it passes nearest the texel.
        let a = place(part.line_start.xyz);
        let b = place(part.line_end.xyz);
        let ab = b.xy - a.xy;
        let share = clamp(dot(image - a.xy, ab) / max(dot(ab, ab), 1e-9), 0.0, 1.0);
        var z = max(mix(a.z, b.z, share), 0.1);
        var was = vec3(0.0);
        var origin = vec3(0.0);
        var lands = vec3(0.0);
        for (var step = 0; step < 3; step++) {
            // Where what is here now was, what the image has there, and where that is now.
            was = place((part.backward * vec4(point(image, z), 1.0)).xyz);
            origin = point(was.xy, metres_at(was.xy, size));
            lands = place((part.forward * vec4(origin, 1.0)).xyz);
            z = lands.z;
        }
        let good = all(was.xy > vec2(0.0)) && all(was.xy < vec2(1.0)) && metres_at(was.xy, size) > 0.0 && belongs(part, origin)
            && all(abs(lands.xy - image) < 1.5 / vec2<f32>(size)) && z > 0.05;
        if good && z < best {
            best = z;
        }
    }
    if best >= none {
        // Something left and nothing came: the farthest of what stays put about here.
        var farthest = 0.0;
        for (var reach = 4; reach <= 32; reach *= 2) {
            for (var way = 0; way < 8; way++) {
                let dir = array<vec2<f32>, 8>(vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(-1.0, -1.0))[way];
                let there = image + dir * f32(reach) / vec2<f32>(size);
                let m = metres_at(there, size);
                if m > farthest && !moves_at(point(there, m)) {
                    farthest = m;
                }
            }
        }
        if farthest <= 0.0 {
            return 1.0;
        }
        best = farthest;
    }
    return depth_of(best);
}
