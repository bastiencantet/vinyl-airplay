# Setup guide

## 1. Connect a real line-level input

A turntable cartridge normally needs a phono preamplifier. If your turntable
has a PHONO/LINE switch, choose LINE when connecting to a line-level input.
A passive RCA-to-jack cable changes the connector, not the signal level.

Prefer a stereo USB **line-input** interface. A cheap USB microphone socket may
be mono, have bias voltage, or clip with a line-level signal. A headphone socket
is not automatically an input. Check the device's documentation.

Start with a modest speaker volume. Gain is fixed digital multiplication:
it increases noise too and can clip. It cannot fix an absent electrical signal.

## 2. Check Linux capture

Install `alsa-utils` using your distribution's package manager. Then run:

```sh
arecord -l
arecord -L
```

Use the actual ALSA identifier in all commands. For a USB interface it may resemble
`plughw:CARD=USB,DEV=0`; that example is not universal. Other applications can
hold the input open. Check access to `/dev/snd` and your distribution's audio
permissions; do not run the sender as root to work around a permission problem.

The sender requests signed stereo 16-bit PCM at 44.1 kHz. `plughw` can adapt
hardware formats. Exact conversion and buffer sizes depend on ALSA and hardware.

## 3. Discover and try one receiver

```sh
./target/release/vinyl-airplay discover --seconds 5
./target/release/vinyl-airplay probe 192.168.50.20:7000 --authenticate
./target/release/vinyl-airplay stream \
  --alsa-device 'plughw:CARD=USB,DEV=0' \
  --target 192.168.50.20:7000 --latency-ms 250
```

Replace the example address and port with the `_airplay._tcp` announcement.
The device's access settings must allow the implemented authentication method.
PIN-based persistent pairing is not supported. The sender never changes those
settings or opens firewall ports for you.

Stop with Ctrl-C. Confirm clean playback with one receiver before trying several.
Requested scheduling is not acoustic latency. Test higher requested delays if
playback is unstable, but also inspect input clipping and network conditions.

## 4. Web interface

```sh
python3 scripts/serve-web.py --alsa-device 'plughw:CARD=USB,DEV=0'
```

Visit http://127.0.0.1:8787/. Select speakers, apply the selection, then start.
The launcher starts one controller, which owns one audio process. Ctrl-C closes
both. Do not run a separate CLI stream against the same capture interface.

Settings and logs live in `~/.local/state/vinyl-airplay/control/`. The controller
token has mode 0600. The gateway injects it server-side. A fresh installation
starts with playback and recognition disabled, no selected receivers, 0 dB gain,
and a requested schedule of 250 ms. Existing settings are retained on restart.

To use a phone, pass a specific private IPv4 to `--listen`. This is a trusted-LAN
interface without individual user login or transport TLS. Everyone who can
reach that address on your trusted network can control playback. The gateway
validates Host and write-request headers; this does not replace network trust.
Do not expose the port publicly. HTTPS/reverse-proxy and service packaging are
future work rather than prerequisites for the local developer preview.

Developer overrides: `--state`, `--controller`, `--sender`, and `--port`.
The standalone controller accepts `VINYL_ROOT`, `VINYL_TOKEN_FILE`, `VINYL_BIND`,
`VINYL_ALSA_DEVICE`, and `VINYL_SENDER_BIN`.

## 5. Optional song identification

```sh
cargo build --release --locked --manifest-path tools/shazam-fingerprint/Cargo.toml
cp tools/shazam-fingerprint/target/release/vinyl-shazam-fingerprint target/release/
```

Install `curl`, then enable recognition in the web UI or add `--recognize` to
the CLI stream. The helper is discovered beside the sender; alternatively set
`VINYL_SHAZAM_FINGERPRINT` to its path. See the source for supported overrides.

This integration uses an unofficial Shazam endpoint and may stop working. It
sends audio fingerprints to that external service and downloads returned artwork;
recognition is optional and failures should not interrupt audio. The project
makes no promise of unlimited or permanently available third-party service.

## Troubleshooting

| Symptom | First checks |
|---|---|
| No receivers | Same LAN, multicast/mDNS reachability, no Wi-Fi client isolation. |
| Receiver discovered but cannot connect | Access settings, receiver firmware, unsupported PIN pairing. |
| Silence or very low input | LINE/PHONO position, correct physical input and ALSA device, capture mute. |
| Distortion | Input clipping, microphone-vs-line mismatch, excessive digital gain. |
| Intermittent clicks | Inspect capture separately, Wi-Fi conditions, requested delay and logs. |
| Never disconnects | Surface noise can exceed the silence threshold. Auto-connect measures level, not platter rotation. |
| Image absent | Optional Apple image service may not carry that model/color; a generic icon is used. |
| No title | Enable recognition, install helper and curl, check external network access. |

Share the smallest redacted log needed for diagnosis. Do not attach music recordings.
