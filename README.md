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

## Quick start

Requires **Rust/Cargo** and **Python 3.10+**. Linux also needs the ALSA development
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
python3 scripts/serve-web.py
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
- [Français](docs/README.fr.md)

## Community

[Report a bug](https://github.com/bastiencantet/vinyl-airplay/issues/new/choose),
[ask a question](https://github.com/bastiencantet/vinyl-airplay/discussions), or
read the [contributing guide](CONTRIBUTING.md) to get involved.
See the [roadmap](ROADMAP.md) for current priorities.

## License

[GPL-3.0-or-later](LICENSE). Third-party attribution is listed in [NOTICE](NOTICE).
This project is independent of Apple and is not an officially certified AirPlay product.
