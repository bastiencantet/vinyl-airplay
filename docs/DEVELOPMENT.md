# Development

Rust stable with edition 2024 support. Linux is the live-capture target. Python
3.10+ is used for the optional web gateway and test emulators, not by the sender.

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
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

- `src/`: sender, pairing, encrypted transport, timing, metadata and input handling.
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
