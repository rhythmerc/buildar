// THUG2's 2D as the game draws it (Direct3D 9, render_pc): a texture times the vertices' colour, doubled, worked in the
// game's own (gamma) values, then blended. The texture is sampled as sRGB (linear here): back to gamma first.
#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct Params {
    // x: the factor (2: doubled), y: textured, z: opaque
    mode: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: Params;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var picture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var picture_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var c = in.color;
    if (params.mode.y > 0.5) {
        let t = textureSample(picture, picture_sampler, in.uv);
        c = vec4(c.rgb * pow(max(t.rgb, vec3(0.0)), vec3(1.0 / 2.2)), c.a * t.a) * params.mode.x;
    }
    c = clamp(c, vec4(0.0), vec4(1.0));
    if (params.mode.z > 0.5) {
        c.a = 1.0;
    }
    return vec4(pow(c.rgb, vec3(2.2)), c.a);
}
