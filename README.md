<img src="assets/icon.svg" alt="" align="right" width="64" height="64">

# Vinyl AirPlay

[![Checks](https://github.com/bastiencantet/vinyl-airplay/actions/workflows/check.yml/badge.svg)](https://github.com/bastiencantet/vinyl-airplay/actions/workflows/check.yml)
[![License: GPL v3+](https://img.shields.io/badge/license-GPL_v3%2B-blue.svg)](LICENSE)

Stream a turntable or other line-level audio source from Linux to HomePod and
HomePod mini. Vinyl AirPlay is written in Rust and includes a local web interface.
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

Requires **Linux**, **Rust/Cargo**, **alsa-utils** and **Python 3.10+**.
Connect a stereo line input and keep the speakers on the same local network.
See [installation](docs/INSTALLATION.md) for wiring and prerequisites.

```sh
git clone https://github.com/bastiencantet/vinyl-airplay.git
cd vinyl-airplay
cargo build --release --locked
arecord -L
```

Choose your input from `arecord -L`, then launch the web interface. Replace
`YOUR_ALSA_INPUT` with that identifier:

```sh
python3 scripts/serve-web.py --alsa-device YOUR_ALSA_INPUT
```

Open **http://localhost:8787/** on the Linux computer. Select a speaker, apply
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
