# Synthetic spectrum reference

`spectrum-reference.pcm` contains 4096 stereo frames of 44.1 kHz signed
16-bit big-endian synthetic PCM. It is generated from a 1 kHz tone with
opposite channel phases, 73 Hz and 12.5 kHz tones, and seeded low-level noise.
It contains no recording or personal audio.

`spectrum-reference.json` contains the original visualizer's reference values:
symmetric Hann window, stereo mean power, real 4096-point FFT, single-sided
power normalization and 20 geometrically spaced bands from 32 Hz to 20 kHz.
Silence is floored at -120 dBFS. The Rust implementation must match every
band within 0.00001 dB on this fixture. Python/NumPy was used only to generate
the independent reference; it is not required to run these tests or the app.
