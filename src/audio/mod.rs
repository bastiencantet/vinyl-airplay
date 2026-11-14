//! Native capture produces the sender's PCM S16BE / 44.1 kHz / stereo contract.
//! The host backend is selected by the target at compile time, never by user OS flags.
#[cfg(feature = "native-audio")]
mod native;
#[cfg(feature = "native-audio")]
pub use native::{Capture, devices};

#[cfg(not(feature = "native-audio"))]
pub fn devices() -> anyhow::Result<serde_json::Value> {
    anyhow::bail!("native audio was excluded; rebuild with the native-audio feature")
}
