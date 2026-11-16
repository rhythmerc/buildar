//! The pad packet the sim's input reads. The headset's Touch controllers supply it
//! ([`super::ExternalPad`]); the desktop's raw XInput transport is gone.
pub(crate) use skate3_sim::input::device::{DeviceError, DevicePacket};
