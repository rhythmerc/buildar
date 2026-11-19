// An eye's picture to the headset's image as it is (occlusion.rs): where the eyes' last pass writes the headset's
// image itself (in place of Bevy's own copy, skipped for them), this is it on a frame occlusion doesn't run.
@group(0) @binding(0) var scene_color: texture_2d<f32>;

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(scene_color, vec2<i32>(position.xy), 0);
}
