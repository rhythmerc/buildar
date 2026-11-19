// Which of the depth image's texels are sure: well inside a surface, with
// no step in depth of more than 8% between neighbours within two texels.
// (At an edge the image is a texel or three out; only sure texels are
// asked what colour the near things and the far things are.)
// The steps are found first (occlusion_steps.wgsl: each texel's largest to
// its neighbours); a texel is sure when none of the 5x5 about it steps more.
// Out: metres along the view; negative where not sure.
#import buildar::xr::occlusion_look::{look, metres}

@group(0) @binding(0) var room_depth: texture_depth_2d;
@group(0) @binding(1) var steps: texture_2d<f32>;

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(room_depth));
    let centre = vec2<i32>(position.xy);
    var most = 0.0;
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            most = max(most, textureLoad(steps, clamp(centre + vec2(x, y), vec2(0), size - 1), 0).x);
        }
    }
    let sure = most <= log(1.08);
    let z = metres(textureLoad(room_depth, centre, 0));
    let d = select(z, 1000.0, z <= 0.0);
    return vec4(select(-d, d, sure), 0.0, 0.0, 1.0);
}
