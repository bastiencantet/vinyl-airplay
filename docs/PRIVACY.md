# Privacy

Vinyl AirPlay has no account, advertising, analytics or usage-reporting service.
Audio capture starts only when playback is enabled. With automatic connection,
the app keeps capturing to detect activity while enabled, including silence;
stop playback or quit to close the input.

## Local operation

Discovery uses Bonjour/mDNS on your network. Captured audio is sent to the
receivers you select. The desktop window communicates directly with its local
Rust controller; it opens no HTTP control port. Settings, a bounded diagnostic
log and the most recent recognized artwork are kept in the OS app-data
directory. They can include device names, addresses and track titles. They are
not uploaded automatically. Do not attach unredacted logs to public issues.

The UI may fetch device icons from Apple's `statici.icloud.com` CDN using the
discovered model and enclosure color. That request exposes your network's public
IP address and the requested icon to the CDN. A missing icon does not stop audio.

## Optional music recognition

Recognition is off by default. Enabling it computes a fingerprint locally from
about 12 seconds of captured sound, then sends the fingerprint to Shazam's
recognition endpoint. This path sends a fingerprint, not the raw PCM recording.
It also downloads matching album artwork. Those third-party services receive
your public IP address and request metadata and apply their own policies.

Recognition is best effort, with retries and backoff. This project promises no
unlimited quota or service availability. Turning it off keeps playback local
and removes those recognition requests. Previously downloaded artwork may
remain in local state until replaced or deleted.
