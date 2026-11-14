# Installation

[Documentation](README.md) / Installation

The alpha is installed from source. Prebuilt releases and a Raspberry Pi image
are not available yet. Check [compatibility](COMPATIBILITY.md) before buying hardware.

## Requirements

- Linux with access to the audio input through ALSA.
- Rust/Cargo with edition 2024 support.
- `alsa-utils` for capture and device discovery.
- Python 3.10+ for the web interface.
- A HomePod or HomePod mini on the same local network.

Install system dependencies with your distribution's package manager. The
application itself runs as your user; it does not need root or Kubernetes.

## Connect the source

```text
Turntable → phono preamp → stereo line input → Linux → HomePod
```

A turntable with a built-in preamp can connect directly to the line input;
set its output to **LINE**. Otherwise, use a phono preamp between the turntable
and input. A passive RCA-to-jack cable does not replace a preamp.

Use a stereo line-input interface. Microphone and headphone sockets are not
interchangeable with a line input; check your interface's specifications.

## Build

```sh
git clone https://github.com/bastiencantet/vinyl-airplay.git
cd vinyl-airplay
cargo build --release --locked
```

This builds the sender and controller in `target/release/`.

## Select an input

```sh
arecord -l
arecord -L
```

`arecord -l` lists capture hardware; `arecord -L` lists ALSA device identifiers.
Choose the identifier for the connected line input. It may look like
`plughw:CARD=USB,DEV=0`, but the name depends on your hardware.

If capture is denied, check your distribution's audio permissions and whether
another application is using the device. See [troubleshooting](TROUBLESHOOTING.md).

## Launch

Replace `YOUR_ALSA_INPUT` with the identifier you selected:

```sh
python3 scripts/serve-web.py --alsa-device YOUR_ALSA_INPUT
```

Open **http://localhost:8787/** on the Linux computer. Playback is disabled on
a fresh installation. Choose a speaker and follow the [usage guide](USAGE.md).

For phone access, see [network configuration](CONFIGURATION.md#network-access).
