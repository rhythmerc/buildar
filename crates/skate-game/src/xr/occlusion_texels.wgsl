// Each depth texel's colour in the camera's newest frame (0-255), found by
// where its point of the room is in that frame. Alpha: its metres; negative
// where it is not sure (occlusion_sure.wgsl) or not in the frame.
#import skate_game::xr::occlusion_look::{look, room_point, in_camera}

@group(0) @binding(0) var sure: texture_2d<f32>;
@group(0) @binding(1) var guide: texture_2d<f32>;
@group(0) @binding(2) var linear: sampler;

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(sure));
    let z = textureLoad(sure, vec2<i32>(position.xy), 0).x;
    let at = in_camera(room_point(position.xy / size, abs(z)));
    let colour = textureSampleLevel(guide, linear, at.xy, 0.0).rgb * 255.0;
    return vec4(colour, select(-abs(z), z, at.z > 0.5));
}
