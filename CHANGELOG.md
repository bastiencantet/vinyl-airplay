# Changelog

## Unreleased — community alpha

- Add a Tauri desktop alpha with bundled native sender/fingerprint helper,
  input selection, local controls, persistent settings and tray lifecycle.
- Share the controller between desktop IPC and the authenticated web adapter.
- Let inactive settings be saved without a speaker; add native input discovery
  to both interfaces and suspend desktop polling when its window is hidden.

- Share the sender as a Rust library and select native capture at compile time:
  ALSA on Linux, Core Audio on macOS, WASAPI on Windows.
- Add native input enumeration, mono/stereo conversion and off-callback sinc
  resampling to the sender's PCM format; retain optional legacy ALSA capture.
- Port local control to Windows named pipes with graceful sender shutdown.
- Extend automated build and protocol checks to Windows; physical macOS/Windows
  capture validation remains pending.

- Extract the Rust audio sender from a working personal Linux installation.
- Provide local web control, live input meters and optional track identification.
- Discover receivers and choose product images from model/color metadata.
- Replace machine-specific capture/mixer settings with explicit input selection.
- Add a loopback-first web launcher, contribution guides and compatibility reporting.
- Exclude private captures, personal infrastructure and experimental TV drivers.

No packaged stable release has been published yet.
