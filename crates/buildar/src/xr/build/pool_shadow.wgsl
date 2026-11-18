// The rim's shadow inside a pool (xr/build/pool_shadow.rs): each point traced toward the light to the mouth's level;
// where that misses the mouth (its picture, `mask`), the rim is in the way. Black at a coverage, premultiplied, drawn
// over the pool's own faces (the same mesh: equal depth passes).
#import bevy_pbr::forward_io::VertexOutput

struct Params {
    local_from_world: mat4x4<f32>,
    // xyz: toward the light, in the pool's own space; w: how dark the shadow is
    light: vec4<f32>,
    // xyz: toward the light in the world (for which faces face it)
    light_world: vec4<f32>,
    // the mask's rectangle: min x, z, then size x, z
    mask_rect: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: Params;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var mask: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var mask_sampler: sampler;

fn open_at(p: vec3<f32>, l: vec3<f32>, spread: f32) -> f32 {
    let t = -p.y / l.y;
    let q = p.xz + l.xz * t;
    let uv = (q - params.mask_rect.xy) / params.mask_rect.zw;
    if any(uv < vec2(0.0)) || any(uv > vec2(1.0)) {
        return 0.0;
    }
    // (softer the further the rim is: four reads round the point)
    let r = spread * t / params.mask_rect.zw;
    var sum = 0.0;
    sum += textureSampleLevel(mask, mask_sampler, uv + vec2(r.x, 0.0), 0.0).r;
    sum += textureSampleLevel(mask, mask_sampler, uv - vec2(r.x, 0.0), 0.0).r;
    sum += textureSampleLevel(mask, mask_sampler, uv + vec2(0.0, r.y), 0.0).r;
    sum += textureSampleLevel(mask, mask_sampler, uv - vec2(0.0, r.y), 0.0).r;
    return sum * 0.25;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = (params.local_from_world * vec4(in.world_position.xyz, 1.0)).xyz;
    let l = params.light.xyz;
    // (under the mouth only, with the light above it)
    if p.y > -0.002 || l.y < 0.05 {
        discard;
    }
    let lit = open_at(p, l, 0.04);
    // (a face turned from the light is dark by its own shading already)
    let facing = saturate(dot(normalize(in.world_normal), params.light_world.xyz) * 3.0);
    let a = (1.0 - lit) * params.light.w * facing;
    if a < 0.004 {
        discard;
    }
    return vec4(0.0, 0.0, 0.0, a);
}
