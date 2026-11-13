# Contributing

Thank you for helping turn a personal prototype into dependable community software.
English and French reports are welcome.

## Start here

1. Read the compatibility matrix and search existing issues.
2. For a bug, provide a minimal reproduction and redacted logs.
3. For new receiver/input support, use the compatibility report form.
4. For a substantial change, discuss the approach before writing a large patch.

Small, focused pull requests are easier to validate. Include what changed, why,
how it was checked and any remaining hardware uncertainty. Tests that prove a
real behavior are welcome; tests that merely repeat implementation details are not.

See [development](docs/DEVELOPMENT.md) for the verification commands.

## Protect users and their equipment

- Do not change volume unexpectedly or start sound automatically in tests.
- Never require public network exposure for discovery or playback.
- Keep the real-time audio path independent of artwork and recognition failures.
- Do not bundle recordings, captures, tokens, private network identities or firmware.
- Redact discovery dumps: addresses, MACs, serial numbers, names and pairing keys.
- Do not claim measured latency when only the requested delay is known.

## License and conduct

By submitting your own contributions, you agree to license them under this
project's GPL-3.0-or-later terms. You must have the right to contribute the code.
Keep upstream attribution and license notices intact.

Be respectful, specific and patient. Critique behavior and code rather than people.
Harassment and publication of another person's private information are not accepted.
There is no guaranteed response time or free individual installation service.
