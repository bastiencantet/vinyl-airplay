# Community roadmap

These are priorities, not promised dates. Help and field reports determine scope.

## First: make the current path reproducible

- [x] Independent Rust sender and protocol emulators.
- [x] Portable input selection and local web launcher.
- [x] Disabled playback and recognition on a fresh installation.
- [x] Dynamic receivers and metadata-based Apple product images.
- [ ] Validate the community extraction on several independent installations.
- [ ] Raspberry Pi test matrix with recommended stereo line-input interfaces.
- [ ] Long-session audio and network/restart recovery tests.
- [ ] Measure acoustic latency and drift using a reproducible method.

## Portable engine

- [x] Shared Rust library and compile-selected native capture backends.
- [x] Native input discovery and source-independent activity/gain path.
- [ ] Physical input and playback validation on macOS and Windows.
- [ ] Device hotplug, sleep/wake and long-session clock-drift handling.
- [ ] Audio-file decoding and OS-specific system-audio capture.

## Then: make installation easier

- [ ] Signed or checksummed release artifacts for validated Linux architectures.
- [ ] User-service packaging and guided input diagnostics.
- [ ] Clear compatibility for stereo pairs and additional receiver firmware.
- [ ] Localization beyond the current French web interface.
- [ ] Home Assistant integration based on the local API.

## Research track

- [ ] Persistent pairing and wider receiver compatibility.
- [ ] Optional TV display with portable packaging and explicit resource controls.

No mandatory account, subscription, remote audio relay or cloud-based controller
is planned for the community audio path. Optional third-party recognition remains
separate from basic playback.
