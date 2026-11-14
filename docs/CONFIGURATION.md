# Configuration

[Documentation](README.md) / Configuration

## Playback settings

On Linux, the web launcher saves settings in `~/.local/state/vinyl-airplay/control/config.json`
(or under `XDG_STATE_HOME`). macOS uses `~/Library/Application Support/vinyl-airplay-web`;
Windows uses `%LOCALAPPDATA%/vinyl-airplay-web`. Use `--state` to select another directory.
Existing settings are restored when the launcher restarts.

| Setting | Default | Behavior |
|---|---|---|
| Input device | System default | Native input ID, saved with playback settings |
| Playback | Off | Start after selecting a speaker |
| Automatic connection | On | Connect on input activity; disconnect after silence |
| Speakers | None | Select from discovered devices |
| Input gain | 0 dB | Fixed digital gain; web range −12 to +12 dB |
| Requested delay | 250 ms | Web range 70–2000 ms; not measured acoustic latency |
| Song recognition | Off | Optional external service; see below |

## Network access

By default, the launcher is accessible only from the computer running it.
`localhost` and `127.0.0.1` both refer to that computer.

To use a phone on the same trusted Wi-Fi, replace `LINUX_HOST_IP` with the Linux
computer's private IPv4 address:

```sh
./target/release/vinyl-web \
  --alsa-device YOUR_ALSA_INPUT --listen LINUX_HOST_IP
```

Open `http://LINUX_HOST_IP:8787/` on the phone, using the same address.

Anyone who can reach that endpoint on your trusted network can control playback.
It uses HTTP without individual user login. Do not forward the port to the
Internet. The controller runs in the launcher process; there is no proxy or second
HTTP control port. Host and Origin checks protect local browser access.

## Song recognition

Install `curl` and build the optional fingerprint helper:

```sh
cargo build --release --locked --manifest-path tools/shazam-fingerprint/Cargo.toml
cp tools/shazam-fingerprint/target/release/vinyl-shazam-fingerprint target/release/
```

Enable recognition in the web interface, or add `--recognize` to a CLI stream.
The helper is found beside the sender. To use another location, set
`VINYL_SHAZAM_FINGERPRINT` to the helper's path.

Recognition sends fingerprints to an unofficial Shazam endpoint and downloads
track artwork. It requires Internet access and may become unavailable. Basic
playback works without it. Product images also use an external Apple service
when an image is available for the discovered model and color.

## Launcher options

| Option | Purpose |
|---|---|
| `--input-device` | Override saved input; otherwise restore the saved/default native input |
| `--alsa-device` | Legacy Linux ALSA input; mutually exclusive with native capture |
| `--listen` | Specific private IPv4; loopback by default |
| `--port` | Web port; defaults to 8787 |
| `--state` | Settings and log directory |
| `--sender` | Path to the sender binary |

For the controller's environment variables and development setup, see
[development](DEVELOPMENT.md).
