//! Skate 3's HUD: the retail trick display (`trickdisplay`, an APT movie run by `skate3_sim::hud_runtime`) as the
//! contract's [`HudFrame`]. Read from the player's prepared HUD (`private/hud`: the movie as JSON, its shapes' and
//! fonts' textures as raw RGBA), advanced with the scoring each tick, and drawn (`apt_scene::draw`) into one part:
//! the trick display's corner of the 1280x720 stage ([`AREA`]), shown on a panel by the player's chosen controller.
//!
//! Its draws are Flash-style: a texture times a colour transform ([`Shading::Transform`]), over what's under it.
use engine_core::glam::Vec2;
use engine_core::hud::{HudBlend, HudDraw, HudFrame, HudPart, HudVertex, Placement, Rgba8, Shading, TextureId};
use skate3_sim::{apt_scene, hud_runtime};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The movie's stage, its units.
pub const STAGE: [f32; 2] = [1280.0, 720.0];
/// The part of the stage shown (x, y from its top left, width, height): the trick display's corner, bottom left (its
/// tricks, the multiplier, the score and the line's, with room above for a long line's tricks and to the right for
/// long names).
pub const AREA: [f32; 4] = [40.0, 360.0, 640.0, 360.0];
/// How many pixels a stage unit is drawn at (the part's resolution).
pub const PIXELS_PER_UNIT: f32 = 1.5;
/// The panel's width, real metres.
pub const WIDTH_M: f32 = 0.2;
/// Where the panel's bottom left is from the grip, real metres (right, up).
pub const OFFSET: Vec2 = Vec2::new(-0.055, 0.02);

/// Where the player's prepared HUD is: `SKATE_SCORING_HUD_ROOT`, or `private/hud` in the game's assets.
pub fn root(asset_root: &Path) -> PathBuf {
    std::env::var_os("SKATE_SCORING_HUD_ROOT").map(PathBuf::from).unwrap_or_else(|| asset_root.join("private/hud"))
}

/// The trick display: its movie running, its shapes, and its textures (by id: their index).
pub struct Hud {
    runtime: hud_runtime::Runtime,
    shapes: apt_scene::Shapes,
    /// Each texture's file, its id its index.
    ids: BTreeMap<String, TextureId>,
    /// Their pixels, until fetched ([`Hud::texture`]).
    textures: Vec<Option<Rgba8>>,
    /// It stopped (an error advancing or drawing it): it shows nothing from then on.
    failed: bool,
    /// Advanced since it was last drawn (or not drawn yet).
    changed: bool,
}

impl Hud {
    /// Reads the HUD from `root` (all but the GPU's part, so it can be done off the main thread: it takes seconds),
    /// its movie started with the scoring's `input`.
    pub fn load(root: &Path, input: hud_runtime::Input) -> Result<Hud, String> {
        let source: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("runtime/trickdisplay.json")).map_err(|e| format!("{}: {e}", root.display()))?,
        )
        .map_err(|e| e.to_string())?;
        let runtime = hud_runtime::Runtime::load(&source, input)?;
        let shapes: apt_scene::Shapes = serde_json::from_value(source["shapes"].clone()).map_err(|e| e.to_string())?;
        let mut files = BTreeMap::new();
        for shape in shapes.values().flatten() {
            files.insert(shape.texture.rgba.clone(), [shape.texture.width, shape.texture.height]);
        }
        for font in runtime.bindings.movie.text_assets.fonts.values() {
            files.insert(font.texture.clone(), font.size);
        }
        let mut ids = BTreeMap::new();
        let mut textures = Vec::new();
        for (path, size) in files {
            let pixels = std::fs::read(root.join(&path)).map_err(|e| format!("HUD {path}: {e}"))?;
            if pixels.len() != size[0] as usize * size[1] as usize * 4 {
                return Err(format!("Invalid HUD texture size {path}"));
            }
            ids.insert(path, textures.len() as TextureId);
            textures.push(Some(Rgba8 { width: size[0], height: size[1], pixels }));
        }
        Ok(Hud { runtime, shapes, ids, textures, failed: false, changed: true })
    }

    /// A tick of the scoring: the movie advanced with its `input` and what happened (a new trick, the trick modified,
    /// the line's tricks closed). An error stops it (and is returned once).
    pub fn advance(&mut self, input: hud_runtime::Input, new: bool, modified: bool, close: bool) -> Result<(), String> {
        if self.failed {
            return Ok(());
        }
        self.changed = true;
        self.runtime.update(input, new, modified, close).inspect_err(|_| self.failed = true)
    }

    /// The trick display as it is now, into `out` (its buffers kept), if it has advanced since it was last drawn:
    /// whether it has. An error drawing it stops it (and is returned once; the frame is then empty).
    pub fn frame(&mut self, out: &mut HudFrame) -> Result<bool, String> {
        if !std::mem::replace(&mut self.changed, false) {
            return Ok(false);
        }
        out.clear();
        out.canvas = Vec2::from_array(STAGE);
        out.parts.push(part());
        if self.failed {
            return Ok(true);
        }
        let draws = match apt_scene::draw(&self.runtime.bindings.movie, &self.runtime.vm, &self.shapes) {
            Ok(draws) => draws,
            Err(error) => {
                self.failed = true;
                return Err(error);
            }
        };
        for draw in draws {
            let Some(&texture) = self.ids.get(&draw.texture) else { continue };
            let first = out.vertices.len() as u32;
            out.vertices.extend(draw.vertices.iter().map(|v| HudVertex {
                position: Vec2::from_array(v.position),
                uv: Vec2::from_array(v.uv),
                color: [255; 4],
            }));
            out.draws.push(HudDraw {
                first,
                count: draw.vertices.len() as u32,
                texture: Some(texture),
                part: 0,
                blend: HudBlend::Alpha,
                shading: Shading::Transform { multiply: draw.multiply, add: draw.add },
            });
        }
        Ok(true)
    }

    /// A texture's pixels, handed over once (none: not one of its ids, or fetched already).
    pub fn texture(&mut self, id: TextureId) -> Option<Rgba8> {
        self.textures.get_mut(id as usize)?.take()
    }
}

/// The one part: the trick display's corner of the stage, on a panel by the controller (its bottom left a little left
/// of the grip and up).
pub fn part() -> HudPart {
    let [x, y, width, height] = AREA;
    HudPart {
        name: "trick display".into(),
        size: Vec2::new(width, height),
        view: Vec2::new(x + width / 2.0, y + height / 2.0),
        resolution: PIXELS_PER_UNIT,
        empty: false,
        placement: Placement::Hand {
            metres_per_px: WIDTH_M / width,
            offset: OFFSET,
            anchor: Vec2::new(-0.5, -0.5),
            hand: None,
            on_left: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The trick display's part: its corner of the stage at its pixels a unit, a panel `WIDTH_M` wide by the hand.
    #[test]
    fn the_part_is_the_trick_display_corner_by_the_hand() {
        let part = part();
        assert_eq!(part.size, Vec2::new(640.0, 360.0));
        assert_eq!(part.view, Vec2::new(360.0, 540.0));
        assert_eq!(part.resolution, 1.5);
        let Placement::Hand { metres_per_px, offset, anchor, hand, on_left } = part.placement else { panic!() };
        assert!((metres_per_px * part.size.x - WIDTH_M).abs() < 1e-6);
        assert_eq!((offset, anchor, hand, on_left), (OFFSET, Vec2::new(-0.5, -0.5), None, None));
    }

    /// Without its files it says where it looked.
    #[test]
    fn a_missing_hud_says_where() {
        let root = Path::new("/nonexistent/hud");
        let input = hud_runtime::Input {
            sequence_score: 0,
            line_score: 0,
            sequence_timer: 0,
            line_time: 0.0,
            line_capacity: 0.0,
            multiplier: 1.0,
            clean: false,
            sketchy: false,
            stance: [false; 4],
            trick_name: String::new(),
            trick_metrics: Default::default(),
            context_tricks: Vec::new(),
        };
        let error = Hud::load(root, input).err().expect("no HUD there");
        assert!(error.contains("/nonexistent/hud"), "{error}");
    }
}
