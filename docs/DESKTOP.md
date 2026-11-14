# Desktop app

[Documentation](README.md) / Desktop app

The Tauri desktop app embeds the Rust sender and recognition helper. It uses
native audio inputs and a local Rust controller, without Python or a web server.
This is an unsigned alpha; there is no signed stable installer yet.

## Download an alpha candidate

Open [Desktop checks](https://github.com/bastiencantet/vinyl-airplay/actions/workflows/desktop.yml),
choose a successful run on `main`, then download the artifact for your computer.
GitHub requires sign-in for workflow artifacts; they expire after 30 days. A run
on a pull-request branch is a development candidate, not a stable release.

| Computer | Artifact | Installer |
|---|---|---|
| Mac with Apple Silicon | `vinyl-airplay-macos-arm64-alpha` | `.dmg` |
| Mac with Intel CPU | `vinyl-airplay-macos-x64-alpha` | `.dmg` |
| Windows x64 | `vinyl-airplay-windows-x64-alpha` | setup `.exe` |
| Linux x64 | `vinyl-airplay-linux-x64-alpha` | `.deb` or `.AppImage` |

Extract the artifact first. Every archive includes `SHA256SUMS.txt`, source commit
and architecture in `build.json`, this guide and license notices. Check the hash
of the installer before opening it:

```sh
# macOS
shasum -a 256 -c SHA256SUMS.txt
# Linux
sha256sum -c SHA256SUMS.txt
```

On Windows, run `Get-FileHash .\<installer-name>.exe -Algorithm SHA256` in
PowerShell and compare with `SHA256SUMS.txt`. Hashes check download integrity;
these candidates have no publisher signature.

- **macOS 11+:** open the DMG and copy Vinyl AirPlay to Applications. macOS may
  block an unsigned app. Only approve a build you trust through the system's
  Privacy & Security settings; do not disable Gatekeeper globally. Allow
  microphone and local-network access when prompted.
- **Windows 10/11 x64:** run the installer. SmartScreen may flag the unverified
  publisher. Audio capture must be allowed in microphone privacy settings;
  firewall permissions on the private LAN are needed for discovery and playback.
- **Linux x64:** use the `.deb` on compatible Debian/Ubuntu systems, or mark the
  AppImage executable and launch it. Packages are built on Ubuntu 22.04;
  compatibility with every distribution is not established. AppImage may need
  the distribution's FUSE support. A graphical session, working audio input and
  tray support are needed. Headless machines should use the CLI/web launcher.

Raspberry Pi and Linux ARM64 desktop installers are not part of this matrix.
The current app interface is in French. See [privacy](PRIVACY.md) before enabling
optional recognition.

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
  the host (`curl --version`); recognition uses the same optional third-party
  service as the CLI. It is disabled on a fresh installation. Playback and
  metering work without it.
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
