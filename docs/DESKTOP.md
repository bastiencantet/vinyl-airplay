# Desktop app

[Documentation](README.md) / Desktop app

The Tauri desktop app embeds the Rust sender and recognition helper. It uses
native audio inputs and a local Rust controller, without Python or a web server.
This is an unsigned alpha; there is no signed stable installer yet.

## Build and run

Install Rust, Node.js 22+ and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)
for your OS. Linux also needs ALSA development headers. In the repository root:

```sh
npm ci
npm run desktop:dev
```

The preparation script builds only host-native sidecars. The app chooses Core
Audio on macOS, WASAPI on Windows or ALSA on Linux at compile time.

To create a release bundle:

```sh
npm run desktop:build
```

Tauri writes bundles under `src-tauri/target/release/bundle/`, or under
`CARGO_TARGET_DIR/release/bundle/` when that variable is set. `--debug` bundles
are for testing, not signed production releases. Signing and notarization are
not configured in this alpha.

## Use

1. Choose the input connected to your source. Refresh after plugging in hardware.
2. Select AirPlay speakers discovered on the same local network.
3. Start playback. The OS may ask for audio-input permission.
4. Adjust receiver volume, input gain, activity detection and requested delay.

Settings can be saved before any speaker is available. Playback starts disabled
on a fresh installation. The input remains closed until playback is enabled;
automatic connection then monitors it and releases speakers after silence.

Closing the window keeps the app running. Its tray/menu-bar icon can reopen the
window, stop playback or quit. Quitting stops the sender; hiding the window
suspends UI polling. Opening a second instance focuses the existing app.

## Scope and state

- Inputs: mono/stereo line input, USB interface or microphone. Audio files and
  system/application loopback are not implemented.
- Recognition: the fingerprint helper is bundled. `curl` must be available on
  the host; recognition uses the same optional third-party service as the CLI.
- Storage: the OS app-data directory for `org.vinylairplay.desktop`. Settings are
  separate from the web launcher's state directory.
- Control: the bundled UI invokes a fixed list of Rust operations. It receives
  no shell access, API token or generic filesystem access. No HTTP control port
  is opened by the desktop app.
- Compatibility: graphical macOS startup and controls are checked separately
  from CI compilation. Windows/Linux GUI behavior and physical audio capture
  still need field validation; see [compatibility](COMPATIBILITY.md).

The GUI and bundled engine use the repository's GPL-3.0-or-later license. License
and attribution files are included in the app resources. This alpha contains no
payment, activation or account requirement.
