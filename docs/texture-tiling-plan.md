# Hiding texture tiling on park pieces (plan)

Large ramps and pools show the repeat of their tiling textures (visible even on the small concrete pool).
Research only so far (2026-11-16); nothing here is built. Costs are estimates, not measured on the Quest.
Line numbers refer to the tree as of that date.

## Recommendation, best first

1. **Concrete and plaster (the pool): hex-tiling plus a macro tone. Build first.**
   - Mikkelsen's hex-tiling: per hex cell a random offset and rotation, three taps blended, then a
     low-frequency brightness multiplier.
   - Cost: 3 colour taps plus about 60 ALU, against 1 tap today; only on concrete/plaster pixels.
   - Concrete046 has no structure (best case). Contrast-restoring curve r about 0.65-0.75. Close to full
     Heitz-Neyret histogram preservation without its precomputed T / T^-1 textures, so skip Heitz-Neyret.
2. **Brick: no stochastic blending** (it would cut through mortar lines; Bricks051 has 14 courses per copy
   with irregular head joints). Instead multiply a hashed tone per course (`floor(v*14)` mixed with the
   cell index, about 10 ALU) and the macro tone. 1 tap.
3. **Wood: macro tone only.** Plywood is already split into sheets with their own UV shift and tone
   (`xr/build/dress.rs:196-206`); hex blending would smear the one-way grain, and rotation is wrong for wood.
4. **Per-piece UV offset: defer.** Meshes and materials are cached per kind and size
   (`xr/build/mod.rs:2141-2147`, `2173`), so it needs a seed in the cache key or per-instance data, and it
   does nothing for repetition within one surface (the pool's problem).
5. **Larger source textures: cheap complement.** CREDITS.json says the ambientCG sources were 1024 px,
   downscaled to 512. The 1024 versions over twice the metres halve the repeat frequency at the same texel
   density (about 5.3 MB per texture with mips). Do it for concrete alongside step 1.

## Mip and derivative pitfalls

- The per-cell offset jumps at cell edges, so implicit derivatives pick a wrong mip in 2x2 quads that
  straddle an edge (seams). The paper uses `SampleGrad`.
- Cheaper on mobile: compute the LOD once in uniform control flow,
  `lod = 0.5*log2(max(dot(dx,dx), dot(dy,dy))) + log2(512)` with `dx = dpdx(uv)`, `dy = dpdy(uv)`, and use
  `textureSampleLevel` for all 3 taps. Rotation doesn't change it; the sampler has no anisotropy
  (`mod.rs:2304-2311`), so nothing is lost.
- Apply `uv_transform` before taking derivatives (`pbr_input.material.uv_transform`,
  `vendor/bevy_pbr/src/render/pbr_fragment.wgsl:123`).

## Implementation outline

- **Material:** `ExtendedMaterial<StandardMaterial, Untiled>` (works on Bevy 0.18). Copy the pattern of
  `SkaterMaterial` (`src/customiser_material.rs:10-55`, full fragment in `src/customiser_stamp.wgsl`).
  Leave `base_color_texture: None` (stops Bevy's own tap) and multiply the result into base colour after
  `pbr_input_from_standard_material`, so lighting, room light, tint, vertex colours, the pool's baked sky
  shading (`pool::sky_shades`) and `NEUTRAL_LEVEL` scaling all keep working. Opaque, so the default
  prepass is fine.
- **New `xr/build/untiled.rs` + `untiled.wgsl`:**

  ```rust
  #[derive(Asset, AsBindGroup, Reflect, Clone)]
  struct Untiled { #[texture(100)] #[sampler(101)] tex: Option<Handle<Image>>,
                   #[uniform(102)] p: Vec4 } // mode (0 off, 1 hex, 2 brick, 3 macro), hex density, contrast r, macro strength
  ```

  ```wgsl
  var pbr = pbr_input_from_standard_material(in, is_front);
  let uv = (pbr.material.uv_transform * vec3(in.uv, 1.)).xy;
  let lod = mip_from(dpdx(uv), dpdy(uv));            // uniform flow, before any branch
  var c = vec3(1.0);
  if mode == 1 {                                       // Deliot-Heitz triangle grid
    let st = mat2x2(1., 0., -0.57735, 1.1547) * uv * p.y;
    // 3 lattice ids + barycentrics; per vertex: s_i = textureSampleLevel(tex, smp, rot(h_i)*uv + hash2(v_i), lod).rgb
    // w_i = pow(w_i, 7) * (0.4 + 0.6*luma(s_i))      // gamma 7, beta 0.6 (paper sec. 3)
    // c = sum(w_i*s_i) / sum(w_i); c = contrast(c, p.z)
  } else if mode == 2 { c = sample(uv, lod) * (0.9 + 0.2*hash1(vec2(floor(uv.y*14.), floor(uv.x)))); }
  else if mode == 3 { c = sample(uv, lod); }
  c *= mix(1., 0.8 + 0.4*value_noise(uv*0.23), p.w); // macro tone, ALU only
  pbr.material.base_color = vec4(pbr.material.base_color.rgb * c, pbr.material.base_color.a);
  // then the stock apply_pbr_lighting / post-lighting tail, as in customiser_stamp.wgsl
  ```

- **`xr/build/mod.rs`:** `PieceAssets.solid` (~2128) and `fn solid` (~2165-2212) return
  `Handle<PieceMaterial>`; the texture moves into the extension and the mode comes from `Finish` (Steel and
  Paint: mode 0, so `solid` keeps one type). Spawn sites: the placed visual (~2575-2590) branches on
  `hidden` (StandardMaterial) vs `solid` (new type), so two spawn arms; also ~2528, ~3021, ~3089, with
  `ResMut<Assets<PieceMaterial>>`. The palette icon re-insert (~2744) never changes type. Register the
  plugin beside `PoolShadowPlugin` (~580, ~3057). `tiling_texture` (~2265) unchanged except optionally
  loading 1024 px concrete.
- `xr/build/pool.rs` (coping, tiles, drain materials) needs no changes.

## Risks

- Type split: anything querying or swapping `MeshMaterial3d<StandardMaterial>` on placed pieces must handle
  both types (only the hidden/solid branch found).
- The extension WGSL carries a copy of Bevy's pbr fragment entry: re-sync on Bevy upgrades.
- Cost on the Quest is unmeasured. Fallback if 3 taps are too much: Quilez's noise-index method (2 taps).

## Sources

- [Mikkelsen 2022, Practical Real-Time Hex-Tiling (JCGT)](https://jcgt.org/published/0011/03/05/paper.pdf)
- [mmikk/hextile-demo](https://github.com/mmikk/hextile-demo)
- [Heitz & Neyret 2018](https://hal.archives-ouvertes.fr/LJK/hal-01824773v1)
- [Godot port of hex-tiling](https://godotshaders.com/shader/stochastic-hex-tiling-mikkelsens-adaptation/)
- [Godot adaptation of Quilez's noise-index method](https://godotshaders.com/?p=12496)
