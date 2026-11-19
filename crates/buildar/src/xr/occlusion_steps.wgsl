// Each texel of the depth image's largest step in depth to its four neighbours, as the difference of the logs of
// their distances (unknown: far away). occlusion_sure.wgsl reads it about each texel.
// (It was one pass reading the 7x7 about each texel - 49 loads and logs a texel, into an array the GPU kept in
// memory - and cost half a millisecond an eye on the headset for a 320x320 image.)
#import buildar::xr::occlusion_look::{look, metres}

@group(0) @binding(0) var room_depth: texture_depth_2d;

fn log_at(at: vec2<i32>, size: vec2<i32>) -> f32 {
    let z = metres(textureLoad(room_depth, clamp(at, vec2(0), size - 1), 0));
    return log(select(z, 1000.0, z <= 0.0));
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(room_depth));
    let at = vec2<i32>(position.xy);
    let here = log_at(at, size);
    let most = max(
        max(abs(here - log_at(at - vec2(1, 0), size)), abs(here - log_at(at + vec2(1, 0), size))),
        max(abs(here - log_at(at - vec2(0, 1), size)), abs(here - log_at(at + vec2(0, 1), size))),
    );
    return vec4(most, 0.0, 0.0, 1.0);
}
