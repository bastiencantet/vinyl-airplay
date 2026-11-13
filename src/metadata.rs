//! DMAP and MediaRemote labels for a live analog source; no track identification or fake duration.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub key: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub artwork: Option<Vec<u8>>,
}

impl Default for Track {
    fn default() -> Self {
        Self {
            key: "vinyl".into(),
            title: "Vinyle".into(),
            artist: "Entrée audio".into(),
            album: "Entrée ligne".into(),
            artwork: None,
        }
    }
}

impl Track {
    pub fn dmap(&self) -> Vec<u8> {
        container(
            b"mlit",
            &[
                tag(b"minm", &self.title),
                tag(b"asar", &self.artist),
                tag(b"asal", &self.album),
            ]
            .concat(),
        )
    }
}

pub fn live_vinyl() -> Vec<u8> {
    Track::default().dmap()
}

pub fn track_update(track: &Track, paused: bool) -> plist::Value {
    use plist::Value as V;
    use sha2::{Digest, Sha256};
    let id = u64::from_be_bytes(
        Sha256::digest(track.key.as_bytes())[..8]
            .try_into()
            .unwrap(),
    );
    let mut info = dictionary([
        (
            "kMRMediaRemoteNowPlayingInfoTitle",
            track.title.clone().into(),
        ),
        (
            "kMRMediaRemoteNowPlayingInfoArtist",
            track.artist.clone().into(),
        ),
        (
            "kMRMediaRemoteNowPlayingInfoAlbum",
            track.album.clone().into(),
        ),
        (
            "kMRMediaRemoteNowPlayingInfoPlaybackRate",
            V::Real(if paused { 0.0 } else { 1.0 }),
        ),
        (
            "kMRMediaRemoteNowPlayingInfoDefaultPlaybackRate",
            V::Real(1.0),
        ),
        (
            "kMRMediaRemoteNowPlayingInfoTimestamp",
            V::Date(std::time::SystemTime::now().into()),
        ),
        (
            "kMRMediaRemoteNowPlayingInfoMediaType",
            "MRMediaRemoteMediaTypeMusic".into(),
        ),
        ("kMRMediaRemoteNowPlayingInfoUniqueIdentifier", id.into()),
        ("kMRMediaRemoteNowPlayingInfoIsLiveStream", true.into()),
    ]);
    if let Some(artwork) = &track.artwork {
        let fields = info.as_dictionary_mut().unwrap();
        fields.insert(
            "kMRMediaRemoteNowPlayingInfoArtworkIdentifier".into(),
            track.key.clone().into(),
        );
        fields.insert(
            "kMRMediaRemoteNowPlayingInfoArtworkMIMEType".into(),
            "image/jpeg".into(),
        );
        fields.insert(
            "kMRMediaRemoteNowPlayingInfoArtworkData".into(),
            V::Data(artwork.clone()),
        );
    }
    command(
        "updateMRNowPlayingInfo",
        dictionary([
            ("type", "npi-text".into()),
            ("mergePolicy", "replace".into()),
            ("params", info),
        ]),
    )
}

fn tag(name: &[u8; 4], value: &str) -> Vec<u8> {
    container(name, value.as_bytes())
}

fn container(name: &[u8; 4], value: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8 + value.len());
    bytes.extend_from_slice(name);
    bytes.extend_from_slice(&(value.len() as u32).to_be_bytes());
    bytes.extend_from_slice(value);
    bytes
}

// MediaRemote /command payloads, researched by OwnTone (airplay.c).
// Only play/pause for the live input: no seek or track change.
pub fn media_remote(device_uid: &str, session: &str, group: &str) -> Vec<plist::Value> {
    use plist::Value as V;
    let mut info = Vec::new();
    pb_bytes(&mut info, 1, session.as_bytes());
    pb_bytes(&mut info, 2, b"Vinyl AirPlay Rust");
    pb_bytes(&mut info, 3, b"Linux vinyl input");
    pb_bytes(&mut info, 4, env!("CARGO_PKG_VERSION").as_bytes());
    pb_bytes(&mut info, 5, b"com.apple.Music");
    for (field, value) in [(7, 1), (8, 139), (22, 1)] {
        pb_uint(&mut info, field, value);
    }
    pb_bytes(&mut info, 12, b"com.apple.Music");
    pb_bytes(&mut info, 19, device_uid.as_bytes());
    pb_bytes(&mut info, 41, session.as_bytes());
    pb_bytes(&mut info, 42, group.as_bytes());
    let mut message = Vec::new();
    pb_uint(&mut message, 1, 15);
    pb_bytes(&mut message, 20, &info);
    pb_bytes(
        &mut message,
        85,
        uuid::Uuid::new_v4().to_string().as_bytes(),
    );
    let mut framed = Vec::new();
    varint(&mut framed, message.len() as u64);
    framed.extend(message);
    let device = dictionary([("params", dictionary([("data", V::Data(framed))]))]);
    let text = track_update(&Track::default(), false);
    let supported = command(
        "updateMRSupportedCommands",
        dictionary([(
            "mrSupportedCommandsFromSender",
            V::Array(
                [0u64, 1, 2]
                    .into_iter()
                    .map(|id| {
                        let entry = dictionary([
                            ("kCommandInfoCommandKey", id.into()),
                            ("kCommandInfoEnabledKey", true.into()),
                        ]);
                        let mut bytes = Vec::new();
                        entry.to_writer_binary(&mut bytes).expect("in-memory plist");
                        V::Data(bytes)
                    })
                    .collect(),
            ),
        )]),
    );
    let playback = playback_state(false);
    let mut client = Vec::new();
    pb_uint(&mut client, 1, std::process::id().into());
    pb_bytes(&mut client, 2, b"com.apple.Music");
    pb_bytes(&mut client, 7, b"Vinyl AirPlay Rust");
    let client = command(
        "updateMRNowPlayingClient",
        dictionary([("mrNowPlayingClient", V::Data(client))]),
    );
    vec![device, text, supported, playback, client]
}

fn dictionary<const N: usize>(entries: [(&str, plist::Value); N]) -> plist::Value {
    plist::Value::Dictionary(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

fn command(kind: &str, params: plist::Value) -> plist::Value {
    dictionary([("type", kind.into()), ("params", params)])
}

fn varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let next = (value & 127) as u8;
        value >>= 7;
        out.push(next | if value == 0 { 0 } else { 128 });
        if value == 0 {
            break;
        }
    }
}

fn pb_uint(out: &mut Vec<u8>, field: u64, value: u64) {
    varint(out, field << 3);
    varint(out, value);
}

fn pb_bytes(out: &mut Vec<u8>, field: u64, value: &[u8]) {
    varint(out, (field << 3) | 2);
    varint(out, value.len() as u64);
    out.extend_from_slice(value);
}

pub fn playback_updates(paused: bool) -> Vec<plist::Value> {
    use plist::Value as V;
    vec![
        command(
            "updateMRNowPlayingInfo",
            dictionary([
                ("type", "npi-text".into()),
                ("mergePolicy", "merge".into()),
                (
                    "params",
                    dictionary([
                        (
                            "kMRMediaRemoteNowPlayingInfoPlaybackRate",
                            V::Real(if paused { 0.0 } else { 1.0 }),
                        ),
                        (
                            "kMRMediaRemoteNowPlayingInfoTimestamp",
                            V::Date(std::time::SystemTime::now().into()),
                        ),
                    ]),
                ),
            ]),
        ),
        playback_state(paused),
    ]
}

pub fn playback_state(paused: bool) -> plist::Value {
    command(
        "updateMRPlaybackState",
        dictionary([("mrPlaybackState", (if paused { 2u64 } else { 1 }).into())]),
    )
}

#[derive(Debug, PartialEq)]
pub enum RemoteCommand {
    Play,
    Pause,
    Toggle,
    Volume(f64),
}

pub fn remote_event(body: &[u8]) -> Option<RemoteCommand> {
    let event: plist::Value = plist::from_bytes(body).ok()?;
    let event = event.as_dictionary()?;
    if event.get("type")?.as_string()? != "sendMediaRemoteCommand" {
        return None;
    }
    match event.get("value")?.as_string()? {
        "play" => Some(RemoteCommand::Play),
        "paus" => Some(RemoteCommand::Pause),
        "plps" => Some(RemoteCommand::Toggle),
        "dvlc" => {
            let value = event
                .get("volume")
                .or_else(|| event.get("params")?.as_dictionary()?.get("volume"))?;
            let level = value
                .as_real()
                .or_else(|| value.as_unsigned_integer().map(|v| v as f64))?;
            (level.is_finite() && (0.0..=1.0).contains(&level))
                .then_some(RemoteCommand::Volume(level))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_volume_accepts_only_finite_normalized_numbers() {
        fn event(value: plist::Value) -> Option<RemoteCommand> {
            let mut b = Vec::new();
            value.to_writer_binary(&mut b).unwrap();
            remote_event(&b)
        }
        for level in [0.0, 0.5, 1.0] {
            assert_eq!(
                event(dictionary([
                    ("type", "sendMediaRemoteCommand".into()),
                    ("value", "dvlc".into()),
                    ("params", dictionary([("volume", level.into())]))
                ])),
                Some(RemoteCommand::Volume(level))
            );
        }
        for level in [-1.0, 1.1, f64::NAN, f64::INFINITY] {
            assert_eq!(
                event(dictionary([
                    ("type", "sendMediaRemoteCommand".into()),
                    ("value", "dvlc".into()),
                    ("volume", level.into())
                ])),
                None
            );
        }
    }
    #[test]
    fn strings_use_utf8_byte_lengths() {
        assert_eq!(
            tag(b"minm", "Été"),
            [b"minm\0\0\0\x05".as_slice(), "Été".as_bytes()].concat()
        );
        let body = live_vinyl();
        assert_eq!(&body[..4], b"mlit");
        assert_eq!(
            u32::from_be_bytes(body[4..8].try_into().unwrap()) as usize,
            body.len() - 8
        );
    }
}
