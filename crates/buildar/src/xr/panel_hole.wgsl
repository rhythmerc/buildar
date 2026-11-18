// The menu panel's place in the scene, where the panel itself is a compositor layer under it (panel_layer.rs): what
// the scene drew there is taken out, by as much as the panel covers it, down to clear (colour and alpha: the
// projection layer's alpha is premultiplied), so the layer shows through. Blended over (panel_layer.rs sets the
// blend to keep `1 - alpha` of what's there): what's drawn after - the lasers, Meta's keyboard - is over it.
#import bevy_pbr::forward_io::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var picture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var picture_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let a = textureSample(picture, picture_sampler, in.uv).a;
    if a < 0.004 {
        discard;
    }
    return vec4(0.0, 0.0, 0.0, a);
}
