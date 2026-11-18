// An engine's HUD draw (engine_core::hud): its colour from its texture and vertices as its Shading says, then blended
// as its HudBlend says (the pipeline's: over by alpha, or added; opaque here, its alpha 1).
#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct Params {
    // x: Modulate's scale, y: textured, z: opaque, w: the shading (0: Modulate in gamma values, 1: Modulate in linear
    // ones, 2: Transform)
    mode: vec4<f32>,
    // Transform's
    multiply: vec4<f32>,
    add: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: Params;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var picture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var picture_sampler: sampler;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var c: vec4<f32>;
    if (params.mode.w > 1.5) {
        // Transform (a Flash-style colour transform, Skate 3's HUD): the texture times multiply plus add, in linear
        // values; the vertices' colour unused.
        var t = vec4(1.0);
        if (params.mode.y > 0.5) {
            t = textureSample(picture, picture_sampler, in.uv);
        }
        c = clamp(t * params.multiply + params.add, vec4(0.0), vec4(1.0));
        if (params.mode.z > 0.5) {
            c.a = 1.0;
        }
        return c;
    }
    if (params.mode.w < 0.5) {
        // Modulate in gamma values (an old fixed-function game's 2D, THUG2's Direct3D 9): the texture is sampled as
        // sRGB (linear here), so back to gamma first; the vertices' colour is in gamma values as given.
        c = in.color;
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
    // Modulate in linear values: the vertices' colour (sRGB-encoded) made linear, times the texture's, times scale.
    c = vec4(srgb_to_linear(in.color.rgb), in.color.a);
    if (params.mode.y > 0.5) {
        c = c * textureSample(picture, picture_sampler, in.uv) * params.mode.x;
    }
    c = clamp(c, vec4(0.0), vec4(1.0));
    if (params.mode.z > 0.5) {
        c.a = 1.0;
    }
    return c;
}
