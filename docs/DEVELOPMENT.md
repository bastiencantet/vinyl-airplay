# Development

Rust stable with edition 2024 support. Native capture targets Linux, macOS and
Windows. Python
3.10+ is used for the optional web gateway and test emulators, not by the sender.

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo check --locked --all-targets --no-default-features
cargo test --locked
cargo build --locked
node --check web/app.js
python3 -m unittest discover -s tests -p 'test_*.py'
```

For protocol emulators, use a virtual environment:

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install -r tests/requirements.txt
python tests/mock_homepods.py target/debug/vinyl-airplay
python tests/mock_volume.py target/debug/vinyl-airplay
```

On Linux, also run the capture/recognition and supervisor integration tests:

```sh
python tests/mock_activity.py target/debug/vinyl-airplay
python tests/mock_recognition.py target/debug/vinyl-airplay
python tests/mock_control.py target/debug/vinyl-control
```

The emulators use synthetic identities and keys. No HomePod, audio interface,
recording or cloud account is needed. They test protocol behavior, not acoustic
performance. Keep real speaker/network experiments explicit and separate.

## Layout

- `src/lib.rs`: shared Rust sender library, used by the CLI. The API is experimental.
- `src/audio/`: compile-selected CPAL capture, bounded realtime queue and off-callback sinc resampling.
- `src/ipc.rs`: private Unix socket / local-only Windows named-pipe transport.
- `src/`: pairing, encrypted transport, timing, metadata and input handling.
- `src/bin/vinyl-control.rs`: authenticated controller and child-process ownership.
- `src/receivers.rs`: live Bonjour catalog, stable identity and enclosure color.
- `web/`: browser interface and input measurements; no audio stream in the browser.
- `scripts/serve-web.py`: restricted same-origin gateway and local launcher.
- `tools/shazam-fingerprint/`: optional GPL SongRec-derived recognition helper.
- `tests/`: unit-level gateway checks and independent protocol emulators.

## Protocol provenance

OwnTone, pyatv and OpenAirPlay documentation were consulted to understand wire
behavior. The Rust transport does not execute those projects. This repository
does not incorporate OwnTone or pyatv source. The SongRec-derived helper does
include attributed upstream code; preserve its NOTICE and license.

References:
- https://github.com/owntone/owntone-server
- https://github.com/postlund/pyatv
- https://github.com/openairplay/airplay-spec
- https://github.com/marin-m/SongRec

Do not import third-party implementations without reviewing the license and
recording their origin. No Apple firmware or recordings belong in this repository.

## Controller environment

The community launcher sets these automatically. They are useful when running
the controller directly or preparing service packaging.

| Variable | Purpose |
|---|---|
| `VINYL_ROOT` | Runtime state directory |
| `VINYL_TOKEN_FILE` | File containing the API bearer token |
| `VINYL_BIND` | Controller socket address; defaults to loopback |
| `VINYL_INPUT_DEVICE` | Native input ID; defaults to `default` |
| `VINYL_ALSA_DEVICE` | Optional legacy ALSA input; takes precedence over native capture |
| `VINYL_SENDER_BIN` | Sender binary; defaults to `bin/vinyl-airplay` under the state directory |

The launcher keeps the controller on loopback, injects the token server-side,
and exposes only the documented routes through the web gateway.

## Native audio checks

CI builds and tests the sender and controller on Linux, macOS and Windows. Linux
builds require ALSA development headers. `--no-default-features` additionally
checks that headless deployments can exclude native capture. macOS builds link
Core Audio; Windows builds use WASAPI. No ASIO SDK or user-facing OS selector is
required. `cargo tree --target TARGET` shows dependencies for a particular target.

Synthetic audio tests check 44.1 kHz PCM preservation, channel order, mono
duplication, 48 kHz conversion duration/pitch and explicit overrun handling. IPC
roundtrip and volume emulators cover both local transports. These checks do not
establish microphone permissions, acoustic latency or working hardware capture.
