// The matte: over the depth view, at a few times the depth image's size,
// how much each pixel is of the far things of its patch and not the near
// (occlusion.wgsl mixes the two answers by it). Where the patch has both,
// the pixel's colour in the camera's frame is asked: each sure texel about
// it votes for its side, by how like the pixel's its colour is, and that
// is weighed with what the depth image says of the place. The evidence is
// kept from frame to frame (found by where the pixel's point was in the
// last frame's view), fading and clamped, so an edge holds still.
//
// Out: the evidence (log-odds of far), the near things' metres, 1 where
// the patch has both, and the share of far there (0 elsewhere).
#import buildar::xr::occlusion_look::{look, metres, room_point, in_camera}

@group(0) @binding(0) var filtered: texture_2d<f32>;
@group(0) @binding(1) var texels: texture_2d<f32>;
@group(0) @binding(2) var guide: texture_2d<f32>;
@group(0) @binding(3) var kept: texture_2d<f32>;
@group(0) @binding(4) var linear: sampler;

// How unlike two colours can be (0-255 a channel) and still vote as one, and how far (texels) a vote carries.
const COLOUR: f32 = 14.0;
const REACH: f32 = 3.0;
// Votes worth this much are needed before the colours count for much.
const DOUBT: f32 = 0.5;

fn logit(p: f32) -> f32 {
    let q = clamp(p, 1e-4, 1.0 - 1e-4);
    return log(q / (1.0 - q));
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let image = position.xy / vec2<f32>(textureDimensions(kept));
    let size = vec2<f32>(textureDimensions(filtered));
    let s = textureSampleLevel(filtered, linear, image - 0.5 / size, 0.0);
    let band = s.w > 1e-7;
    let near = metres(1.0 - s.x);
    let far = metres(1.0 - s.y);
    // What the depth image says: how far into the far things the patch's mean is.
    let depth_says = smoothstep(0.2, 0.8, s.z / max(s.w, 1e-6));
    var share = depth_says;
    let point = room_point(image, near);
    if band && near > 0.0 {
        // Were the near thing at this pixel, this is where the camera would see it.
        let at = in_camera(point);
        var like = 0.5;
        if at.z > 0.5 {
            let colour = textureSampleLevel(guide, linear, at.xy, 0.0).rgb * 255.0;
            let place = image * size;
            let base = vec2<i32>(floor(place));
            let last = vec2<i32>(size) - 1;
            var for_near = 0.0;
            var for_far = 0.0;
            for (var y = -5; y <= 5; y += 2) {
                for (var x = -5; x <= 5; x += 2) {
                    let texel = clamp(base + vec2(x, y), vec2(0), last);
                    let t = textureLoad(texels, texel, 0);
                    if t.a <= 0.0 {
                        continue;
                    }
                    let unlike = t.rgb - colour;
                    let apart = vec2<f32>(texel) + 0.5 - place;
                    let vote = exp(-dot(unlike, unlike) / (2.0 * COLOUR * COLOUR) - dot(apart, apart) / (2.0 * REACH * REACH));
                    for_near += vote * f32(t.a <= near * 1.12);
                    for_far += vote * f32(far > 0.0 && t.a >= far * 0.88);
                }
            }
            like = (for_near + DOUBT * 0.5) / (for_near + for_far + DOUBT);
        }
        let before = clamp(1.0 - depth_says, 0.1, 0.9);
        let near_share = like * before / max(like * before + (1.0 - like) * (1.0 - before), 1e-9);
        share = 1.0 - near_share;
    }
    var evidence = logit(share);
    if band && near > 0.0 && (look.flags & 2u) != 0u {
        // Where the pixel's point (on the near thing: an edge moves with what is in front) was in the old view.
        let old = (look.old_view_from_tracking * vec4(point, 1.0)).xyz;
        let ahead = -old.z;
        let t = look.old_tangents;
        let old_image = vec2((old.x / ahead - t.x) / (t.y - t.x), (old.y / ahead - t.z) / (t.w - t.z));
        if ahead > 0.05 && all(old_image > vec2(0.0)) && all(old_image < vec2(1.0)) {
            let was = textureSampleLevel(kept, linear, old_image, 0.0);
            // (the old view had the same thing in front there; else something moved: start again)
            if was.y > 0.0 && abs(ahead / was.y - 1.0) < 0.08 {
                evidence += look.keep * was.x;
            }
        }
    }
    evidence = clamp(evidence, -look.most, look.most);
    let is_band = f32(band);
    return vec4(evidence, near, is_band, is_band / (1.0 + exp(-evidence)));
}
