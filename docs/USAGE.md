# Usage

[Documentation](README.md) / Usage

## Web interface

Start the launcher from the project directory:

```sh
python3 scripts/serve-web.py --alsa-device YOUR_ALSA_INPUT
```

Open **http://localhost:8787/** on the same computer.

1. Select one or more discovered speakers.
2. Apply the selection.
3. Start playback and play a record.

The speaker's volume slider acts immediately. Applying changes to the selected
speakers, input gain or requested delay reconnects the audio stream.

Stop with the web button, or press Ctrl-C in the terminal to close the launcher,
controller and capture process. Do not run a separate CLI stream using the same
input while the web controller is running.

## Automatic connection

With automatic connection enabled, the input stays open while the sender waits
for audio. It connects after sustained activity and disconnects after silence.
Surface noise can keep the connection active: this detects audio level, not
whether the turntable is switched on.

The beginning of a track can be missed during detection and connection.

## Levels and gain

Meters show the captured input before digital gain. Start at **0 dB** and adjust
only if necessary. Positive gain also amplifies noise and can clip louder passages.
It cannot repair a missing or already distorted input signal.

Speaker volume and input gain are separate controls.

## Command line

The sender can also run without the web interface. First discover receivers:

```sh
./target/release/vinyl-airplay discover --seconds 5
```

Use the address and port advertised by `_airplay._tcp` in place of `RECEIVER_ADDRESS`:

```sh
./target/release/vinyl-airplay stream \
  --alsa-device YOUR_ALSA_INPUT \
  --target RECEIVER_ADDRESS \
  --auto-connect --latency-ms 250
```

Repeat `--target` for additional receivers. Ctrl-C stops playback and closes
sessions. The requested delay is scheduling configuration, not a measurement
of the delay heard at the speaker.

For authentication diagnostics, without playing audio:

```sh
./target/release/vinyl-airplay probe RECEIVER_ADDRESS --authenticate
```

See `./target/release/vinyl-airplay --help` and each subcommand's `--help` for
all options. Redact receiver identities before sharing discovery or probe output.
