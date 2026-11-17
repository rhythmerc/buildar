//! The XR options, from the command line (`--xr [--xr-scale K]`); always on in the headset.
/// OpenXR view options. Physics never sees these; they only scale the view.
#[derive(Clone, Copy, Debug)]
pub(crate) struct XrOptions {
    /// Real-world size / miniature size. 12 makes a 1.8 m skater 15 cm tall.
    pub scale: f32,
}

impl Default for XrOptions {
    fn default() -> Self {
        Self { scale: 12.0 }
    }
}
