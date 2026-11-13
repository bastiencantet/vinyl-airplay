# Compatibility and evidence

“Tested” below means the original author exercised physical equipment. It does
not mean a continuous compatibility lab, certification, or every firmware build.

| Component | Evidence | Limits |
|---|---|---|
| HomePod, first generation (`AudioAccessory1,1`) | Audible playback and web volume on a physical unit | Longer independent field tests needed. |
| HomePod mini (`AudioAccessory5,1`) | Audible playback and web volume on a physical unit | Short requested-delay trials are not end-to-end latency measurements. |
| Two independent HomePods | Common sender timing, retransmissions and simultaneous experiments | Acoustic synchronization across rooms not certified. |
| HomePod second generation | Not validated by this project | A matching product image is not proof of audio support. |
| Apple-configured stereo pair | Not validated | Not equivalent to selecting two independent receivers. |
| Linux x86-64 | Physical capture on an older Intel Mac mini running Linux | The community controller intentionally removes its machine-specific mixer workarounds. |
| Linux ARM64 | Sender trials on an M2 Mac mini running Linux | Does not validate Raspberry Pi audio interfaces. |
| Raspberry Pi | Planned community target | No ready-made image or physical test results published yet. |
| macOS | Build and protocol tests | No native live capture implementation; PCM stdin is available. |
| Other AirPlay receivers | Discovery exists | Audio authentication and real-time profiles vary. |
| TV mirroring / hardware AVE | Excluded from community alpha | Personal experimental stack is not a portable product. |

## Audio path

The current live path sends uncompressed stereo PCM, 16-bit / 44.1 kHz. Digital
gain can clip. Analog capture, resampling, receiver DSP and speaker output mean
“no lossy encoding in the sender” does not imply an unchanged analog signal.

## Delay

The CLI accepts a requested schedule; the web interface exposes 70–2000 ms.
Receiver buffering, capture, processing and network conditions affect the result.
Do not describe a selected 90 ms or 100 ms setting as measured acoustic latency.
A low number that works for one short session is not a universal recommendation.

## Add your result

Use the compatibility issue form. Include the model identifier and firmware,
Linux distribution/kernel, CPU architecture, input interface, exact command
with addresses redacted, requested delay, duration and audible outcome.
A successful discovery or handshake alone should not be reported as working audio.
