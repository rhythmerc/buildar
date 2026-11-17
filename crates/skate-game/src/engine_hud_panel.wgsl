// A HUD panel in the room (engine_hud.rs's PanelMaterial): the picture, unlit, drawn over everything, virtual or
// real. The room's occlusion pass (xr/occlusion.wgsl) hides pixels by the scene's depth, so where the panel has paint
// it writes the near plane's depth (1: reverse z), which that pass never hides; with nothing written, a panel over a
// virtual piece that a real thing hid was hidden with it. Where the picture is clear nothing is written, and what is
// behind is hidden or not as before.
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    forward_io::VertexOutput,
    pbr_functions::main_pass_post_lighting_processing,
}

struct PanelOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> PanelOutput {
    let pbr_input = pbr_input_from_standard_material(in, is_front);
    let colour = pbr_input.material.base_color;
    if colour.a < 0.02 {
        discard;
    }
    var out: PanelOutput;
    out.color = main_pass_post_lighting_processing(pbr_input, colour);
    out.depth = 1.0;
    return out;
}
