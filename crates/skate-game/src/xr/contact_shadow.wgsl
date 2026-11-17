// Contact shadows over the room and the park (xr/contact_shadow.rs): black at a coverage, premultiplied, so over
// passthrough the real room behind is darkened by it, and a virtual piece under it the same. Written into depth where
// it shows, so the occlusion pass hides it behind real things as it hides the skater.
#import bevy_pbr::forward_io::VertexOutput

struct Caster {
    // xyz: the middle (world); w: its height over the surface under it
    centre: vec4<f32>,
    // xyz: along it (level, unit); w: half its length
    nose: vec4<f32>,
    // xyz: across it; w: half its width
    side: vec4<f32>,
    // x: darkness, y: how high it fades over, z: how much it spreads with height, w: the skater's size
    shape: vec4<f32>,
}

struct Params {
    // (MAX_CASTERS in contact_shadow.rs)
    casters: array<Caster, 24>,
    count: u32,
    // 0: the casters' shadows; 1: the vertices' alpha (a piece's contact ring)
    mode: u32,
    opacity: f32,
    pad: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: Params;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var a = 0.0;
    if params.mode == 1u {
#ifdef VERTEX_COLORS
        a = in.color.a;
#endif
    } else {
        for (var i = 0u; i < params.count; i++) {
            let c = params.casters[i];
            let d = in.world_position.xyz - c.centre.xyz;
            // (only on what is under it)
            if d.y > 0.1 * c.shape.w {
                continue;
            }
            let h = c.centre.w;
            let grow = 1.0 + c.shape.z * h / c.shape.w;
            let q = vec2(dot(d, c.nose.xyz) / c.nose.w, dot(d, c.side.xyz) / c.side.w) / grow;
            // (outside it: nothing, and no exp to pay for)
            if dot(q, q) >= 1.0 {
                continue;
            }
            let fall = 1.0 - smoothstep(0.3, 1.0, length(q));
            a = max(a, fall * c.shape.x * exp(-h / c.shape.y) / grow);
        }
    }
    a = a * params.opacity;
    // (nothing to darken: not drawn, nor written into depth - it would hide what's behind it from nothing)
    if a < 0.004 {
        discard;
    }
    return vec4(0.0, 0.0, 0.0, a);
}
