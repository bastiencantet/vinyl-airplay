# Installation

[Documentation](README.md) / Installation

The alpha is installed from source. Prebuilt releases and a Raspberry Pi image
are not available yet. Check [compatibility](COMPATIBILITY.md) before buying hardware.

## Requirements

- Linux, macOS or Windows with an accessible audio input.
- Linux: ALSA development headers and `pkg-config` (`libasound2-dev` on Debian/Ubuntu).
- Windows: the Rust MSVC toolchain and Visual Studio C++ Build Tools.
- Rust/Cargo with edition 2024 support.
- `alsa-utils` only for the optional legacy Linux capture path.
- A HomePod or HomePod mini on the same local network.

Install system dependencies with your distribution's package manager. The
application itself runs as your user; it does not need root or Kubernetes.

## Connect the source

```text
Turntable → phono preamp → stereo line input → computer → HomePod
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

This builds the sender, native web launcher and standalone controller adapter in `target/release/`.

## Select an input

```sh
./target/release/vinyl-airplay audio-inputs
```

On Windows, use `target/release/vinyl-airplay.exe`. The JSON lists input names,
IDs, the default input and the compiled host backend. Select an **ID**, not a
list index or a display name. Use `default` to follow the system default at the
next start; the running capture does not automatically switch devices.

Native capture uses Core Audio on macOS, WASAPI on Windows and ALSA on Linux.
It accepts common integer/float mono (duplicated to both channels) or stereo PCM. It prefers 44.1 kHz;
otherwise it converts the device's default rate to 44.1 kHz with a sinc filter.
Capture callbacks never wait for networking or run the resampler. A buffer
overrun or device loss stops the stream with an error rather than hiding sample
loss. Reconnection after hotplug is not implemented yet.

macOS and Windows may require microphone/input permission for the terminal or
application launching the sender. A headphone-only socket cannot capture audio.
System/application loopback and audio-file decoding are not implemented yet.

For the existing Linux `arecord` path, install `alsa-utils`, run `arecord -L`,
and use `--alsa-device YOUR_ALSA_INPUT` instead of `--input-device`.

## Launch

Use the default input:

```sh
./target/release/vinyl-web
```

Pass `--input-device INPUT_ID` to choose another input. On Windows, use
`target/release/vinyl-web.exe`. The UI is embedded in the Rust binary.

Open **http://localhost:8787/** on the computer running the sender. Playback is disabled on
a fresh installation. Choose a speaker and follow the [usage guide](USAGE.md).

For phone access, see [network configuration](CONFIGURATION.md#network-access).

## Headless Linux builds

The native backend is enabled by default. To retain only stdin and the legacy
`arecord` path, without linking the native audio libraries:

```sh
cargo build --release --locked --no-default-features
./target/release/vinyl-web --alsa-device YOUR_ALSA_INPUT
```

This does not include native capture. Each OS needs its own compiled artifact;
one executable does not run on all three systems.
