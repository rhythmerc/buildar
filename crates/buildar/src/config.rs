//! The XR options, from the command line (`--xr [--xr-scale K]`); always on in the headset.
/// OpenXR view options. Physics never sees these; they only scale the view.
#[derive(Clone, Copy, Debug)]
pub(crate) struct XrOptions {
    /// Real-world size / miniature size: a skate engine's own (`Engine::default_scale`, about 7: a 1.8 m skater 25 cm
    /// tall) until a course says otherwise.
    pub scale: f32,
}

impl Default for XrOptions {
    fn default() -> Self {
        Self { scale: crate::engines::Engine::Skate3.default_scale() }
    }
}
