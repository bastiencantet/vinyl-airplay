<img src="assets/icon.svg" alt="" align="right" width="64" height="64">

# Vinyl AirPlay

[![Checks](https://github.com/bastiencantet/vinyl-airplay/actions/workflows/check.yml/badge.svg)](https://github.com/bastiencantet/vinyl-airplay/actions/workflows/check.yml)
[![License: GPL v3+](https://img.shields.io/badge/license-GPL_v3%2B-blue.svg)](LICENSE)

Stream a turntable, USB audio interface or microphone to HomePod and HomePod mini. Vinyl AirPlay is written in Rust and includes a local web interface.
Your phone controls playback; it does not relay the audio.

**Alpha software.** Tested with HomePod (1st generation) and HomePod mini.
See [compatibility and known limitations](docs/COMPATIBILITY.md).

## Features

- Connect when audio starts and disconnect after silence.
- Discover speakers and control their volume from a browser.
- Adjust input gain and monitor stereo levels and the audio spectrum.
- Optionally identify songs and display album artwork.
- Run locally without an account or subscription.

## Desktop app (alpha)

A native desktop interface lets you choose an audio input,
select discovered speakers and control playback from the app or tray icon.
Download published alpha installers from
[Releases](https://github.com/bastiencantet/vinyl-airplay/releases), or build from source.
Development candidates are also available from successful
[Desktop checks runs](https://github.com/bastiencantet/vinyl-airplay/actions/workflows/desktop.yml)
(GitHub sign-in required for workflow artifacts).
See [desktop installation and scope](docs/DESKTOP.md).
Mac Apple Silicon/Intel, Windows x64 and Linux x64 packages are built automatically;
these are alpha candidates, with field testing still needed on macOS and Windows.

## Quick start

Requires **Rust/Cargo**. Linux also needs the ALSA development
package. Native backends are selected at compile time for Linux, macOS and Windows;
physical capture/playback validation is still Linux-only.
Connect a stereo line input and keep the speakers on the same local network.
See [installation](docs/INSTALLATION.md) for wiring and prerequisites.

```sh
git clone https://github.com/bastiencantet/vinyl-airplay.git
cd vinyl-airplay
cargo build --release --locked
./target/release/vinyl-airplay audio-inputs
```

Launch with the system default input, or select an input ID with `--input-device`:

```sh
./target/release/vinyl-web
```

Open **http://localhost:8787/** on the computer running the sender. Select a speaker, apply
the selection, then start playback. To control it from your phone, follow
[access from another device](docs/CONFIGURATION.md#network-access).

## Documentation

- [Installation](docs/INSTALLATION.md)
- [Usage](docs/USAGE.md)
- [Configuration](docs/CONFIGURATION.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [Compatibility](docs/COMPATIBILITY.md)
- [Privacy](docs/PRIVACY.md)
- [Français](docs/README.fr.md)

## Community

[Report a bug](https://github.com/bastiencantet/vinyl-airplay/issues/new/choose),
[ask a question](https://github.com/bastiencantet/vinyl-airplay/discussions), or
read the [contributing guide](CONTRIBUTING.md) to get involved.
See the [roadmap](ROADMAP.md) for current priorities.

## License

[GPL-3.0-or-later](LICENSE). Third-party attribution is listed in [NOTICE](NOTICE).
This project is independent of Apple and is not an officially certified AirPlay product.
