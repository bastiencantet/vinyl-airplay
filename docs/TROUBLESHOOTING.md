# Troubleshooting

[Documentation](README.md) / Troubleshooting

| Problem | Check |
|---|---|
| No speakers found | Same LAN, multicast/mDNS availability and Wi-Fi client isolation |
| Speaker found but connection fails | Receiver firmware, access settings and [supported authentication](COMPATIBILITY.md) |
| No input audio | Selected native input ID (or legacy ALSA device), OS input permission, cable, capture mute and LINE/PHONO setting |
| Native input unavailable | Run `audio-inputs` again, check OS microphone permission and whether the device was unplugged |
| Native capture overrun/stall | Check device stability and system load; restart after correcting the fault |
| Very low signal | Correct line input and a suitable phono preamp |
| Distortion | Input clipping, microphone-vs-line mismatch and excessive digital gain |
| Intermittent clicks | Capture quality, network stability and requested delay |
| Automatic connection never ends | Surface noise may remain above the silence threshold |
| No track information | Recognition enabled, helper and curl installed, external service reachable |
| Missing product image | The external image service may not carry that model/color |
| Phone cannot open the page | [LAN listening address](CONFIGURATION.md#network-access), same Wi-Fi and host firewall |

## Isolate capture from transport

Check the physical input before changing AirPlay settings. An audio interface
that clips or records a weak signal will pass that problem on to the speakers.
Then try one receiver before testing multiple destinations.

Increasing the requested delay may help a timing problem. It will not fix
analog distortion, capture clipping or noise already present in the record.

## Logs

The launcher prints controller output in its terminal. Capture logs are stored in
`control/audio.log` under the [state directory](CONFIGURATION.md#playback-settings); the web interface also shows
recent output under its technical status panel.

## Get help

Search [existing issues](https://github.com/bastiencantet/vinyl-airplay/issues)
or [open a report](https://github.com/bastiencantet/vinyl-airplay/issues/new/choose).
Include the receiver model/firmware, OS version, input interface, requested
delay, session duration and steps to reproduce.

Share only the relevant log excerpt. Remove tokens, IP/MAC addresses, device
names, serial numbers and pairing information. Do not upload music recordings.
Use [Discussions](https://github.com/bastiencantet/vinyl-airplay/discussions)
for setup questions.
