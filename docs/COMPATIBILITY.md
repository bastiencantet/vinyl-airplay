# Compatibility

[Documentation](README.md) / Compatibility

Tested means audible playback was checked on physical equipment. Automated
protocol tests are listed separately in [development](DEVELOPMENT.md).

## Receivers

| Receiver | Status |
|---|---|
| HomePod, first generation (`AudioAccessory1,1`) | Playback and web volume tested |
| HomePod mini (`AudioAccessory5,1`) | Playback and web volume tested |
| HomePod, second generation | Not tested |
| Apple-configured stereo pair | Not tested |
| Other AirPlay receivers | Discovery supported; playback not verified |

## Hosts

| Platform | Status |
|---|---|
| Linux x86-64 | Capture and playback tested on an Intel Mac mini |
| Linux ARM64 | Sender tested on an M2 Mac mini running Linux |
| Raspberry Pi | Awaiting hardware validation |
| macOS | Build and protocol tests; no native live capture |

The community controller uses the selected ALSA input. It does not apply
machine-specific mixer workarounds from the original personal installation.

## Known limitations

- The live path uses stereo PCM, **16-bit / 44.1 kHz**. No lossy encoding is
  applied by this sender; analog capture, resampling and receiver processing
  still affect the result.
- Requested scheduling is not a measurement of acoustic latency. No universal
  minimum latency is claimed.
- Two independent receivers have been exercised, but long-session acoustic
  synchronization remains to be measured.
- Persistent PIN-based pairing is not implemented. Receiver access settings
  and firmware can affect connection compatibility.
- Automatic connection can miss the start of a track during negotiation.
- Native iPhone/Home volume controls may be unavailable. Use the web slider.
- TV mirroring is outside the current community alpha.

## Report a test

Use the [compatibility report form](https://github.com/bastiencantet/vinyl-airplay/issues/new/choose).
Include the model and firmware, Linux version, CPU architecture, input interface,
requested delay, session duration and audible result. Discovery or a successful
handshake alone does not establish working audio.
