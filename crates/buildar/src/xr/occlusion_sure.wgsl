// Which of the depth image's texels are sure: well inside a surface, with
// no step in depth of more than 8% between neighbours within two texels.
// (At an edge the image is a texel or three out; only sure texels are
// asked what colour the near things and the far things are.)
// Out: metres along the view; negative where not sure.
#import buildar::xr::occlusion_look::{look, metres}

@group(0) @binding(0) var room_depth: texture_depth_2d;

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(room_depth));
    let centre = vec2<i32>(position.xy);
    // The 7x7 about the texel, as logs of distance (unknown: far away).
    var logs: array<f32, 49>;
    for (var i = 0; i < 49; i++) {
        let z = metres(textureLoad(room_depth, clamp(centre + vec2(i % 7 - 3, i / 7 - 3), vec2(0), size - 1), 0));
        logs[i] = log(select(z, 1000.0, z <= 0.0));
    }
    let step = log(1.08);
    var sure = true;
    // Each texel within two of this one against its four neighbours.
    for (var y = 1; y <= 5; y++) {
        for (var x = 1; x <= 5; x++) {
            let here = logs[y * 7 + x];
            let most = max(max(abs(here - logs[y * 7 + x - 1]), abs(here - logs[y * 7 + x + 1])), max(abs(here - logs[(y - 1) * 7 + x]), abs(here - logs[(y + 1) * 7 + x])));
            sure = sure && most <= step;
        }
    }
    let z = exp(logs[24]);
    return vec4(select(-z, z, sure), 0.0, 0.0, 1.0);
}
