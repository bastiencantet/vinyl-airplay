//! Experimental realtime (type 96), PCM16 stereo, NTP AirPlay 2 transport.
//! PTP-only firmware and persistent HomeKit pairing are intentionally unsupported.
use crate::{
    clock::{FRAMES, PACKET_BYTES, RATE, Timeline, timing_reply},
    crypto::{Cipher, derive},
    pairing,
    rtsp::{Channel, Client, DEADLINE},
};
use anyhow::{Context, Result, anyhow, ensure};
use plist::{Dictionary, Value};
use std::{
    collections::VecDeque,
    future::Future,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    net::UdpSocket,
    sync::{Mutex, mpsc, watch},
    task::JoinHandle,
    time::{Instant, MissedTickBehavior, interval, sleep_until},
};

type Backlog = Arc<Mutex<VecDeque<(u16, Vec<u8>)>>>;

pub enum InputSource {
    Stdin,
    Native(String),
    Alsa(String),
    Encoded(Codec),
}

struct InputReader {
    reader: Box<dyn AsyncRead + Unpin + Send>,
    // kill_on_drop stops capture on EOF, Ctrl-C, or protocol failure.
    _capture: Option<tokio::process::Child>,
    live: bool,
    pending_pcm: Vec<u8>,
    filled_pcm: usize,
}

impl InputReader {
    // Preserve partial PCM reads across select! cancellation during setup/teardown.
    async fn packet(&mut self, encoded: bool) -> Result<Option<Vec<u8>>> {
        if encoded {
            return read_source_packet(&mut self.reader, true).await;
        }
        while self.filled_pcm < PACKET_BYTES {
            let n = self
                .reader
                .read(&mut self.pending_pcm[self.filled_pcm..])
                .await?;
            if n == 0 {
                if self.filled_pcm == 0 {
                    return Ok(None);
                }
                ensure!(
                    self.filled_pcm.is_multiple_of(4),
                    "PCM input ended in the middle of a stereo frame"
                );
                self.pending_pcm[self.filled_pcm..].fill(0);
                break;
            }
            self.filled_pcm += n;
        }
        self.filled_pcm = 0;
        Ok(Some(self.pending_pcm.clone()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Codec {
    Pcm,
    Alac,
    AacLc,
    AacEld,
}
impl Codec {
    pub fn rate(self) -> u32 {
        if self == Self::AacEld { 48000 } else { RATE }
    }
    fn frames(self) -> u32 {
        match self {
            Self::Pcm | Self::Alac => FRAMES,
            Self::AacLc => 1024,
            Self::AacEld => 480,
        }
    }
    fn format(self) -> u32 {
        match self {
            Self::Pcm => 0x800,
            Self::Alac => 0x40000,
            Self::AacLc => 0x400000,
            Self::AacEld => 0x2000000,
        }
    }
    fn ct(self) -> u32 {
        match self {
            Self::Pcm => 1,
            Self::Alac => 2,
            Self::AacLc => 3,
            Self::AacEld => 4,
        }
    }
}
impl InputSource {
    fn codec(&self) -> Codec {
        match self {
            Self::Encoded(codec) => *codec,
            _ => Codec::Pcm,
        }
    }

    fn open(self) -> Result<InputReader> {
        match self {
            Self::Stdin | Self::Encoded(_) => Ok(InputReader {
                reader: Box::new(tokio::io::stdin()),
                _capture: None,
                live: false,
                pending_pcm: vec![0; PACKET_BYTES],
                filled_pcm: 0,
            }),
            Self::Native(device) => {
                #[cfg(feature = "native-audio")]
                {
                    Ok(InputReader {
                        reader: Box::new(crate::audio::Capture::open(device)?),
                        _capture: None,
                        live: true,
                        pending_pcm: vec![0; PACKET_BYTES],
                        filled_pcm: 0,
                    })
                }
                #[cfg(not(feature = "native-audio"))]
                {
                    let _ = device;
                    anyhow::bail!("native audio was excluded; rebuild with native-audio");
                }
            }
            Self::Alsa(device) => {
                ensure!(
                    cfg!(target_os = "linux"),
                    "--alsa-device requires Linux; stdin PCM is available on other systems"
                );
                eprintln!(
                    "Starting ALSA capture on {device} (20 ms requested capture buffer; monitor opens before setup)…"
                );
                let mut capture = tokio::process::Command::new("arecord")
                    .args([
                        "-q",
                        "-D",
                        &device,
                        "-t",
                        "raw",
                        "-f",
                        "S16_BE",
                        "-r",
                        "44100",
                        "-c",
                        "2",
                        "--buffer-time=20000",
                        "--period-time=8000",
                    ])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::inherit())
                    .kill_on_drop(true)
                    .spawn()
                    .context("cannot start arecord; install alsa-utils on the Linux sender")?;
                let stdout = capture
                    .stdout
                    .take()
                    .context("missing ALSA capture output")?;
                Ok(InputReader {
                    reader: Box::new(stdout),
                    _capture: Some(capture),
                    live: true,
                    pending_pcm: vec![0; PACKET_BYTES],
                    filled_pcm: 0,
                })
            }
        }
    }
}

fn dict(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Dictionary(
        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect::<Dictionary>(),
    )
}
fn number(value: u32) -> Value {
    Value::Integer(value.into())
}
fn port(value: &Value, key: &str) -> Result<u16> {
    let raw = value
        .as_dictionary()
        .and_then(|d| d.get(key))
        .and_then(Value::as_unsigned_integer)
        .with_context(|| format!("missing {key} in SETUP reply"))?;
    let port = u16::try_from(raw)?;
    ensure!(port != 0, "invalid zero {key}");
    Ok(port)
}

pub struct Session {
    address: SocketAddr,
    session_uuid: String,
    group_uuid: String,
    media_remote: bool,
    paused: watch::Sender<bool>,
    client: Arc<Mutex<Client>>,
    data: UdpSocket,
    cipher: Option<Cipher>,
    backlog: Backlog,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
    errors: mpsc::UnboundedSender<String>,
    pub reported_latency: Option<u32>,
    control: Option<(Arc<UdpSocket>, SocketAddr)>,
    timing_requests: Arc<AtomicU64>,
    last_rtp: Arc<AtomicU64>,
    volume: crate::volume::Level,
}

impl Session {
    fn spawn<F>(&mut self, label: &'static str, job: F)
    where
        F: Future<Output = Result<()>> + Send + 'static,
    {
        let mut stop = self.shutdown.subscribe();
        let errors = self.errors.clone();
        let address = self.address;
        self.tasks.push(tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = stop.changed() => {},
                result = job => {
                    let description = match result { Ok(()) => "channel closed".to_string(), Err(error) => format!("{error:#}") };
                    let _ = errors.send(format!("{address}: {label}: {description}"));
                }
            }
        }));
    }

    pub async fn connect(
        address: SocketAddr,
        latency: u32,
        ports: Option<(u16, u16)>,
        errors: mpsc::UnboundedSender<String>,
        codec: Codec,
    ) -> Result<Self> {
        ensure!(
            address.is_ipv4(),
            "this prototype currently supports IPv4 targets only"
        );
        let client = Client::connect(address).await?;
        let local = SocketAddr::new(client.channel.local_addr()?.ip(), 0);
        let data = UdpSocket::bind(local).await?;
        let (shutdown, _) = watch::channel(false);
        let mut session = Self {
            address,
            session_uuid: uuid::Uuid::new_v4().to_string().to_uppercase(),
            group_uuid: uuid::Uuid::new_v4().to_string().to_uppercase(),
            media_remote: false,
            paused: watch::channel(false).0,
            client: Arc::new(Mutex::new(client)),
            data,
            cipher: None,
            backlog: Arc::new(Mutex::new(VecDeque::new())),
            shutdown,
            tasks: vec![],
            errors,
            reported_latency: None,
            control: None,
            timing_requests: Arc::new(AtomicU64::new(0)),
            last_rtp: Arc::new(AtomicU64::new(0)),
            volume: Arc::new(std::sync::Mutex::new(None)),
        };
        if let Err(error) = session.setup(local, latency, ports, codec).await {
            session.stop().await;
            return Err(error).with_context(|| format!("AirPlay setup for {address}"));
        }
        Ok(session)
    }

    async fn setup(
        &mut self,
        local: SocketAddr,
        latency: u32,
        ports: Option<(u16, u16)>,
        codec: Codec,
    ) -> Result<()> {
        let client_arc = self.client.clone();
        let mut client = client_arc.lock().await;
        let secret = pairing::transient(&mut client)
            .await
            .context("transient pairing (access must permit this sender)")?;
        let info = client
            .request("GET", Some("/info"), None, &[], &[])
            .await?
            .ok()?;
        let info: Value = plist::from_bytes(&info.body).context("receiver /info plist")?;
        if let Some(db) = info
            .as_dictionary()
            .and_then(|i| i.get("initialVolume"))
            .and_then(Value::as_real)
            .filter(|db| crate::volume::valid_db(*db))
        {
            crate::volume::observed(self.address, &self.volume, db);
        }
        self.media_remote = info
            .as_dictionary()
            .and_then(|info| info.get("features"))
            .and_then(Value::as_unsigned_integer)
            .is_some_and(|features| features & (1u64 << 50) != 0);
        let timing = UdpSocket::bind(SocketAddr::new(local.ip(), ports.map_or(0, |p| p.0)))
            .await
            .context("cannot bind incoming NTP port")?;
        let timing_port = timing.local_addr()?.port();
        eprintln!("{}: incoming NTP UDP port {timing_port}", self.address);
        let clock = Timeline::new(0);
        let peer_ip = self.address.ip();
        let timing_requests = self.timing_requests.clone();
        self.spawn("NTP timing", async move {
            let mut bytes = [0; 2048];
            loop {
                let (size, peer) = timing.recv_from(&mut bytes).await?;
                let received = clock.now();
                if peer.ip() != peer_ip {
                    continue;
                }
                if let Some(reply) = timing_reply(&bytes[..size], received, clock.now()) {
                    timing_requests.fetch_add(1, Ordering::Relaxed);
                    timing.send_to(&reply, peer).await?;
                }
            }
        });
        let sender_id = uuid::Uuid::new_v4();
        let mac = sender_id.as_bytes()[..6]
            .iter()
            .enumerate()
            .map(|(i, b)| format!("{:02X}", if i == 0 { (b | 2) & 0xfe } else { *b }))
            .collect::<Vec<_>>()
            .join(":");
        let base = dict([
            ("deviceID", mac.clone().into()),
            ("macAddress", mac.into()),
            ("sessionUUID", self.session_uuid.clone().into()),
            ("groupUUID", self.group_uuid.clone().into()),
            ("name", "Vinyl AirPlay Rust".into()),
            ("model", "RustSender1,1".into()),
            ("timingPort", number(timing_port.into())),
            ("timingProtocol", "NTP".into()),
            ("isMultiSelectAirPlay", true.into()),
            (
                "senderSupportsRelay",
                (self.media_remote && codec == Codec::Pcm).into(),
            ),
            ("groupContainsGroupLeader", false.into()),
        ]);
        let response = client
            .plist("SETUP", &base)
            .await
            .context("NTP session SETUP; firmware requiring PTP is not supported yet")?;
        let event_port = port(&response, "eventPort")?;
        if let Some(fields) = response.as_dictionary() {
            eprintln!(
                "{}: session SETUP returned fields {:?}; NTP timingPort={:?}",
                self.address,
                fields.keys().collect::<Vec<_>>(),
                fields
                    .get("timingPort")
                    .and_then(Value::as_unsigned_integer)
            );
        }
        let mut event = Channel::connect(SocketAddr::new(peer_ip, event_port))
            .await
            .context("event channel connect")?;
        // Receiver-originated event channel: key directions are reversed relative to control.
        event.enable(
            &derive(&secret, "Events-Salt", "Events-Read-Encryption-Key"),
            &derive(&secret, "Events-Salt", "Events-Write-Encryption-Key"),
        );
        let paused = self.paused.clone();
        let volume = self.volume.clone();
        let address = self.address;
        self.spawn("events", async move {
            loop {
                let request = event.read().await?;
                if let Some(command) = crate::metadata::remote_event(&request.body) {
                    match command {
                        crate::metadata::RemoteCommand::Volume(level) => {
                            // The receiver already applied Siri/button volume; observe, never echo it.
                            crate::volume::observed(address, &volume, level * 30.0 - 30.0);
                        }
                        command => {
                            paused.send_modify(|value| {
                                *value = match command {
                                    crate::metadata::RemoteCommand::Play => false,
                                    crate::metadata::RemoteCommand::Pause => true,
                                    crate::metadata::RemoteCommand::Toggle => !*value,
                                    crate::metadata::RemoteCommand::Volume(_) => unreachable!(),
                                }
                            });
                            eprintln!(
                                "AirPlay remote command {command:?}; live input paused={}",
                                *paused.borrow()
                            );
                        }
                    }
                }
                let mut reply =
                    String::from("HTTP/1.1 200 OK\r\nContent-Length: 0\r\nAudio-Latency: 0\r\n");
                if let Some(cseq) = request.headers.get("cseq") {
                    let cseq: u32 = cseq.parse()?;
                    reply.push_str(&format!("CSeq: {cseq}\r\n"));
                }
                reply.push_str("\r\n");
                tokio::time::timeout(DEADLINE, event.write(reply.as_bytes())).await??;
            }
        });
        // OwnTone's HomePod sequence is session SETUP → RECORD → stream SETUP.
        // Tested against firmware 1005.13.2; match the known sender's ordering.
        let record = client.request("RECORD", None, None, &[], &[]).await?.ok()?;
        self.reported_latency = record
            .headers
            .get("audio-latency")
            .map(|s| s.parse())
            .transpose()?;
        let control = Arc::new(
            UdpSocket::bind(SocketAddr::new(local.ip(), ports.map_or(0, |p| p.1)))
                .await
                .context("cannot bind incoming control port")?,
        );
        let control_port = control.local_addr()?.port();
        eprintln!("{}: incoming control UDP port {control_port}", self.address);
        let mut audio_key = [0; 32];
        use rand::RngCore;
        rand::rngs::OsRng.fill_bytes(&mut audio_key);
        let stream = dict([
            ("audioFormat", number(codec.format())),
            ("audioMode", "default".into()),
            ("controlPort", number(control_port.into())),
            ("ct", number(codec.ct())),
            ("isMedia", true.into()),
            ("latencyMin", number(latency)),
            ("latencyMax", number(88200u32.max(latency))),
            ("shk", Value::Data(audio_key.to_vec())),
            ("spf", number(codec.frames())),
            ("sr", number(codec.rate())),
            ("type", number(96)),
            ("supportsDynamicStreamID", false.into()),
            ("streamConnectionID", number(client.id)),
        ]);
        let response = client
            .plist("SETUP", &dict([("streams", Value::Array(vec![stream]))]))
            .await
            .context("realtime stream SETUP")?;
        let stream = response
            .as_dictionary()
            .and_then(|d| d.get("streams"))
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .context("missing stream SETUP response")?;
        if let Some(fields) = stream.as_dictionary() {
            eprintln!(
                "{}: stream SETUP returned fields {:?}",
                self.address,
                fields.keys().collect::<Vec<_>>()
            );
            for key in [
                "arrivalToRenderLatencyMs",
                "audioLatencyMs",
                "latencyMin",
                "latencyMax",
            ] {
                if let Some(value) = fields.get(key) {
                    // Only public numeric timing metadata; never dump stream keys.
                    if let Some(number) = value.as_real() {
                        eprintln!(
                            "{}: receiver {key}={number} (reported, not acoustically measured)",
                            self.address
                        );
                    } else if let Some(number) = value.as_unsigned_integer() {
                        eprintln!(
                            "{}: receiver {key}={number} (reported, not acoustically measured)",
                            self.address
                        );
                    }
                }
            }
        }
        let data_port = port(stream, "dataPort")?;
        let remote_control = SocketAddr::new(peer_ip, port(stream, "controlPort")?);
        self.data
            .connect(SocketAddr::new(peer_ip, data_port))
            .await?;
        self.cipher = Some(Cipher::new(&audio_key));
        // Scope is this authenticated peer; retransmit request counts are bounded.
        let backlog = self.backlog.clone();
        let control_rx = control.clone();
        self.spawn("retransmissions", async move {
            let mut request = [0; 2048];
            loop {
                let (len, peer) = control_rx.recv_from(&mut request).await?;
                if peer != remote_control || len != 8 || request[1] & 0x7f != 0x55 {
                    continue;
                }
                let seq = u16::from_be_bytes(request[4..6].try_into()?);
                let count = u16::from_be_bytes(request[6..8].try_into()?).min(512);
                let cache = backlog.lock().await;
                let packets: Vec<_> = (0..count)
                    .filter_map(|i| cache.iter().find(|(s, _)| *s == seq.wrapping_add(i)))
                    .map(|(s, p)| {
                        let mut reply = vec![0x80, 0xd6];
                        reply.extend_from_slice(&s.to_be_bytes());
                        reply.extend_from_slice(p);
                        reply
                    })
                    .collect();
                drop(cache);
                for packet in packets {
                    control_rx.send_to(&packet, peer).await?;
                }
            }
        });
        // Sync is started later with the SAME timeline for both receivers.
        self.control = Some((control, remote_control));
        Ok(())
    }

    pub fn start(
        &mut self,
        timeline: Timeline,
        latency: u32,
        updates: Option<watch::Receiver<crate::metadata::Track>>,
    ) {
        crate::volume::register(self.address, self.client.clone(), self.volume.clone());
        let (control, remote) = self.control.take().expect("session setup completed");
        self.spawn("sync", async move {
            let mut ticker = interval(Duration::from_secs(1));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            let mut first = true;
            loop {
                ticker.tick().await;
                control
                    .send_to(&timeline.sync(latency, first), remote)
                    .await?;
                first = false;
            }
        });
        let client = self.client.clone();
        let mut stop = self.shutdown.subscribe();
        let errors = self.errors.clone();
        let address = self.address;
        let mut paused = self.paused.subscribe();
        let media_remote = self.media_remote;
        let last_rtp = self.last_rtp.clone();
        let (idle, fallback) = watch::channel(crate::metadata::Track::default());
        let has_recognition = updates.is_some();
        let mut updates = updates.unwrap_or(fallback);
        self.tasks.push(tokio::spawn(async move {
            let _idle = idle;
            let mut ticker = interval(Duration::from_secs(2));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                let changed = tokio::select! {biased;
                    _ = stop.changed() => break,
                    result = paused.changed(), if media_remote => { if result.is_err() { break; } true },
                    result = updates.changed(), if has_recognition => {
                        if result.is_err() { break; }
                        let track = updates.borrow_and_update().clone();
                        let is_paused = *paused.borrow();
                        let rtp = last_rtp.load(Ordering::Relaxed);
                        if let Err(error) = publish_track(&mut *client.lock().await, &track, is_paused, media_remote, rtp).await {
                            // Optional metadata failure must not stop working audio.
                            eprintln!("{address}: recognized metadata unavailable: {error:#}");
                        } else { eprintln!("{address}: recognized metadata published: {} / {} (artwork={})", track.title, track.album, track.artwork.is_some()); }
                        continue;
                    },
                    _ = ticker.tick() => false,
                };
                if changed {
                    let is_paused = *paused.borrow_and_update();
                    for command in crate::metadata::playback_updates(is_paused) {
                        let mut body = Vec::new();
                        if let Err(error) = command.to_writer_binary(&mut body) {
                            let _ = errors.send(format!("{address}: playback state: {error:#}"));
                            return;
                        }
                        let result = client.lock().await.request("POST", Some("/command"), Some("application/x-apple-binary-plist"), &body, &[]).await.and_then(|r| r.ok());
                        if let Err(error) = result {
                            let _ = errors.send(format!("{address}: playback state: {error:#}"));
                            return;
                        }
                    }
                    continue;
                }
                // Finish an in-flight exchange before TEARDOWN reuses this connection.
                let result = client
                    .lock()
                    .await
                    .request("POST", Some("/feedback"), None, &[], &[])
                    .await
                    .and_then(|r| r.ok());
                if let Err(error) = result {
                    let _ = errors.send(format!("{address}: feedback: {error:#}"));
                    break;
                }
            }
        }));
    }

    pub async fn send(&mut self, header: &[u8; 12], pcm: &[u8]) -> Result<()> {
        // Live pause discards incoming audio; resuming never replays an old backlog.
        let muted;
        let pcm = if *self.paused.borrow() {
            muted = vec![0; pcm.len()];
            &muted
        } else {
            pcm
        };
        let packet = audio_packet(
            self.cipher.as_mut().context("audio cipher missing")?,
            header,
            pcm,
        )?;
        let seq = u16::from_be_bytes(header[2..4].try_into()?);
        self.last_rtp.store(
            (u64::from(seq) << 32) | u64::from(u32::from_be_bytes(header[4..8].try_into()?)),
            Ordering::Relaxed,
        );
        let mut cache = self.backlog.lock().await;
        if cache.len() == 512 {
            cache.pop_front();
        }
        cache.push_back((seq, packet.clone()));
        drop(cache);
        self.data.send(&packet).await?;
        Ok(())
    }

    pub async fn stop(&mut self) {
        crate::volume::unregister(self.address);
        eprintln!(
            "{}: valid NTP requests received: {}",
            self.address,
            self.timing_requests.load(Ordering::Relaxed)
        );
        let _ = self.shutdown.send(true);
        for task in self.tasks.drain(..) {
            let _ = task.await;
        }
        let result = self
            .client
            .lock()
            .await
            .request("TEARDOWN", None, None, &[], &[])
            .await;
        if let Err(error) = result {
            eprintln!("{}: teardown: {error:#}", self.address);
        }
    }
}

async fn publish_track(
    client: &mut Client,
    track: &crate::metadata::Track,
    paused: bool,
    media_remote: bool,
    rtp: u64,
) -> Result<()> {
    let info = [(
        "RTP-Info",
        format!("seq={};rtptime={}", rtp >> 32, rtp as u32),
    )];
    client
        .request(
            "SET_PARAMETER",
            None,
            Some("application/x-dmap-tagged"),
            &track.dmap(),
            &info,
        )
        .await?
        .ok()?;
    if let Some(artwork) = &track.artwork {
        let result = client
            .request("SET_PARAMETER", None, Some("image/jpeg"), artwork, &info)
            .await?
            .ok();
        if let Err(error) = result {
            eprintln!("Artwork DMAP unavailable: {error:#}");
        }
    }
    if media_remote {
        let mut body = Vec::new();
        crate::metadata::track_update(track, paused).to_writer_binary(&mut body)?;
        client
            .request(
                "POST",
                Some("/command"),
                Some("application/x-apple-binary-plist"),
                &body,
                &[],
            )
            .await?
            .ok()?;
    }
    Ok(())
}

pub fn audio_packet(cipher: &mut Cipher, header: &[u8; 12], pcm: &[u8]) -> Result<Vec<u8>> {
    let (sealed, nonce) = cipher.seal(pcm, &header[4..12])?;
    let mut packet = Vec::with_capacity(header.len() + sealed.len() + 8);
    packet.extend_from_slice(header);
    packet.extend_from_slice(&sealed);
    packet.extend_from_slice(&nonce);
    Ok(packet)
}

/// Read precisely one packet; reject partial stereo frames instead of dropping bytes.
pub async fn read_packet<R: AsyncRead + Unpin>(input: &mut R) -> Result<Option<Vec<u8>>> {
    let mut buffer = vec![0; PACKET_BYTES];
    let mut filled = 0;
    while filled < buffer.len() {
        let count = input.read(&mut buffer[filled..]).await?;
        if count == 0 {
            break;
        }
        filled += count;
    }
    if filled == 0 {
        return Ok(None);
    }
    ensure!(
        filled % 4 == 0,
        "PCM input ended in the middle of a stereo frame"
    );
    // A partial final packet is padded with silence, not another audio frame.
    Ok(Some(buffer))
}

/// Research input framing: a raw access unit preceded by a u16 big-endian size.
async fn read_source_packet<R: AsyncRead + Unpin>(
    input: &mut R,
    encoded: bool,
) -> Result<Option<Vec<u8>>> {
    if !encoded {
        return read_packet(input).await;
    }
    let mut prefix = [0; 2];
    if input.read(&mut prefix[..1]).await? == 0 {
        return Ok(None);
    }
    input
        .read_exact(&mut prefix[1..])
        .await
        .context("truncated encoded packet length")?;
    let len = usize::from(u16::from_be_bytes(prefix));
    ensure!(
        (1..=4096).contains(&len),
        "encoded access unit length must be 1..4096 bytes"
    );
    let mut frame = vec![0; len];
    input
        .read_exact(&mut frame)
        .await
        .context("truncated encoded access unit")?;
    Ok(Some(frame))
}

#[derive(Default)]
struct SenderMetrics {
    late: VecDeque<u64>,
    packets: u64,
    bytes: u64,
    max_late: u64,
    work_us: u128,
    max_work: u64,
    max_read: u64,
    max_pre_wait_late: u64,
    outliers: VecDeque<serde_json::Value>,
}

impl SenderMetrics {
    fn outlier(&mut self, value: serde_json::Value) {
        if self.outliers.len() == 32 {
            self.outliers.pop_front();
        }
        self.outliers.push_back(value);
    }
    fn record(&mut self, late: u64, work: u64, bytes: usize) {
        if self.late.len() == 4096 {
            self.late.pop_front();
        }
        self.late.push_back(late);
        self.packets += 1;
        self.bytes += bytes as u64;
        self.max_late = self.max_late.max(late);
        self.work_us += u128::from(work);
        self.max_work = self.max_work.max(work);
    }
}

impl Drop for SenderMetrics {
    fn drop(&mut self) {
        if self.packets == 0 {
            return;
        }
        let mut late: Vec<u64> = self.late.iter().copied().collect();
        late.sort_unstable();
        let percentile = |p: usize| late[((late.len() * p).div_ceil(100) - 1).min(late.len() - 1)];
        eprintln!(
            "sender_metrics={}",
            serde_json::json!({
                "packets": self.packets, "payload_bytes": self.bytes,
                "pacing_window_packets": late.len(), "pacing_p50_us": percentile(50),
                "pacing_p95_us": percentile(95), "pacing_p99_us": percentile(99),
                "max_late_us": self.max_late, "work_avg_us": self.work_us / u128::from(self.packets),
                "work_max_us": self.max_work,
                "read_max_us": self.max_read,
                "pre_wait_late_max_us": self.max_pre_wait_late,
                "outliers_last_32": self.outliers,
                "scope": "sender only; excludes capture, live codec encoding, receiver decode, and acoustics"
            })
        );
    }
}

pub async fn play(
    source: InputSource,
    targets: Vec<SocketAddr>,
    latency: u32,
    udp_base_port: Option<u16>,
    gain_db: i16,
    auto_gain: bool,
    recognize: bool,
) -> Result<()> {
    ensure!(
        !auto_gain || gain_db == 0,
        "choose either automatic or fixed input gain"
    );
    ensure!(
        (-60..=60).contains(&gain_db),
        "input gain must be -60..60 dB"
    );
    if let InputSource::Alsa(device) = &source {
        ensure!(cfg!(target_os = "linux"), "--alsa-device requires Linux");
        ensure!(!device.is_empty(), "ALSA device must not be empty");
    }
    ensure!(
        !targets.is_empty() && targets.len() <= 8,
        "specify 1 to 8 targets"
    );
    let mut unique = std::collections::HashSet::new();
    ensure!(
        targets.iter().all(|a| unique.insert(*a)),
        "duplicate receiver target"
    );
    let codec = source.codec();
    let encoded = codec != Codec::Pcm;
    ensure!(
        !encoded || (gain_db == 0 && !auto_gain),
        "preencoded audio cannot use PCM gain"
    );
    let (errors_tx, mut errors_rx) = mpsc::unbounded_channel();
    let mut sessions = Vec::new();
    for (index, target) in targets.into_iter().enumerate() {
        eprintln!(
            "Connecting to {target} (transient pairing, {codec:?}/{}Hz/stereo, NTP)…",
            codec.rate()
        );
        let ports =
            udp_base_port.map(|base| (base + index as u16 * 2, base + index as u16 * 2 + 1));
        match Session::connect(target, latency, ports, errors_tx.clone(), codec).await {
            Ok(session) => sessions.push(session),
            Err(error) => {
                for session in &mut sessions {
                    session.stop().await;
                }
                return Err(error);
            }
        }
    }
    let result = async {
        let mut input = source.open()?;
        run(
            &mut input,
            codec,
            &mut sessions,
            &mut errors_rx,
            RunOptions {
                requested: latency,
                gain_db,
                auto_gain,
                activity: None,
                recognize,
            },
        )
        .await
        .map(|_| ())
    }
    .await;
    for session in &mut sessions {
        session.stop().await;
    }
    result
}

/// Monitor raw input continuously, discarding setup audio to avoid stale buffers.
#[allow(clippy::too_many_arguments)]
pub async fn monitor(
    source: InputSource,
    targets: Vec<SocketAddr>,
    latency: u32,
    udp_base_port: Option<u16>,
    gain_db: i16,
    auto_gain: bool,
    recognize: bool,
    settings: crate::activity::Settings,
) -> Result<()> {
    ensure!(
        matches!(&source, InputSource::Alsa(_) | InputSource::Native(_)),
        "auto-connect requires a live input"
    );
    ensure!(
        !targets.is_empty() && targets.len() <= 8,
        "specify 1 to 8 targets"
    );
    let mut unique = std::collections::HashSet::new();
    ensure!(
        targets.iter().all(|a| unique.insert(*a)),
        "duplicate receiver target"
    );
    let mut input = source.open()?;
    loop {
        eprintln!(
            "Monitoring input: no HomePod session; trigger {} dBFS for 300 ms, release after {} s below {} dBFS.",
            settings.threshold_dbfs,
            settings.silence_seconds,
            settings.off_dbfs()
        );
        let mut detector = crate::activity::Detector::new(settings);
        let mut meter = crate::telemetry::Meter::default();
        loop {
            let packet = tokio::select! {
                _ = crate::shutdown::requested() => return Ok(()),
                packet = input.packet(false) => packet?,
            }
            .context("Live capture ended while monitoring")?;
            meter.observe(&packet);
            if detector.observe(&packet) {
                break;
            }
        }
        eprintln!(
            "Input activity detected; connecting to HomePods. Audio during setup is discarded."
        );
        let (errors_tx, mut errors_rx) = mpsc::unbounded_channel();
        let mut sessions = Vec::new();
        let connect = async {
            for (index, target) in targets.iter().enumerate() {
                let ports = udp_base_port
                    .map(|base| (base + index as u16 * 2, base + index as u16 * 2 + 1));
                sessions.push(
                    Session::connect(*target, latency, ports, errors_tx.clone(), Codec::Pcm)
                        .await?,
                );
            }
            Ok(())
        };
        // Keep draining the sound card while receivers authenticate; no growing backlog.
        let mut interrupted = false;
        let result = tokio::select! {
            result = drain_during(&mut input, connect) => result,
            _ = crate::shutdown::requested() => { interrupted = true; Ok(()) },
        };
        let result = match result {
            _ if interrupted => Ok(RunEnd::Stopped),
            Ok(()) => {
                run(
                    &mut input,
                    Codec::Pcm,
                    &mut sessions,
                    &mut errors_rx,
                    RunOptions {
                        requested: latency,
                        gain_db,
                        auto_gain,
                        activity: Some(settings),
                        recognize,
                    },
                )
                .await
            }
            Err(error) => Err(error),
        };
        let stop = async {
            for session in &mut sessions {
                session.stop().await;
            }
            Ok(())
        };
        drain_during(&mut input, stop).await?;
        match result {
            Ok(RunEnd::Stopped) => return Ok(()),
            Ok(RunEnd::Silence) => {}
            Err(error) => {
                eprintln!("Playback failed: {error:#}; monitoring again after 5 s.");
                tokio::select! {
                    _ = crate::shutdown::requested() => return Ok(()),
                    result = drain_during(&mut input, async {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        Ok(())
                    }) => result?,
                }
            }
        }
    }
}

async fn drain_during<T>(
    input: &mut InputReader,
    future: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::pin!(future);
    loop {
        tokio::select! {
            result = &mut future => return result,
            packet = input.packet(false) => {
                if let Err(error) = packet.and_then(|p| p.context("Live capture ended while connecting/disconnecting")) {
                    // Finish cleanup even if capture fails while TEARDOWN is in flight.
                    let _ = future.await;
                    return Err(error);
                }
            }
        }
    }
}

struct RunOptions {
    requested: u32,
    gain_db: i16,
    auto_gain: bool,
    activity: Option<crate::activity::Settings>,
    recognize: bool,
}

#[derive(PartialEq)]
enum RunEnd {
    Stopped,
    Silence,
}

async fn run(
    input: &mut InputReader,
    codec: Codec,
    sessions: &mut [Session],
    errors: &mut mpsc::UnboundedReceiver<String>,
    options: RunOptions,
) -> Result<RunEnd> {
    let interrupt = crate::shutdown::requested();
    tokio::pin!(interrupt);
    let RunOptions {
        requested,
        gain_db,
        auto_gain,
        activity,
        recognize,
    } = options;
    let mut silence = activity.map(crate::activity::SilenceDetector::new);
    let encoded = codec != Codec::Pcm;
    let rate = codec.rate();
    let frames = codec.frames();
    let mut metrics = SenderMetrics::default();
    let mut gain = if auto_gain {
        eprintln!(
            "Input calibration: measure 5 seconds of signal above -55 dBFS RMS, then hold a fixed stereo gain (max +30 dB). RMS target -20 dBFS, observed-peak headroom 9 dB; no extra audio buffer."
        );
        crate::gain::InputGain::automatic()
    } else {
        eprintln!(
            "Digital input gain: {gain_db:+} dB (0 dB preserves PCM; positive gain amplifies noise and can clip)."
        );
        crate::gain::InputGain::new(gain_db)
    };
    let initial_seq: u16 = rand::random();
    let initial_rtp: u32 = rand::random();
    if input.live {
        // Drain ALSA during control exchanges so metadata cannot add a stale backlog.
        let publish = async {
            for session in sessions.iter_mut() {
                let result = session
                    .client
                    .lock()
                    .await
                    .request(
                        "SET_PARAMETER",
                        None,
                        Some("application/x-dmap-tagged"),
                        &crate::metadata::live_vinyl(),
                        &[(
                            "RTP-Info",
                            format!("seq={initial_seq};rtptime={initial_rtp}"),
                        )],
                    )
                    .await
                    .and_then(|response| response.ok());
                match result {
                    Ok(_) => eprintln!(
                        "{}: Now Playing metadata accepted: Vinyle / Entrée audio",
                        session.address
                    ),
                    Err(error) => eprintln!(
                        "{}: Now Playing metadata unavailable: {error:#}",
                        session.address
                    ),
                }
                if session.media_remote {
                    let mut client = session.client.lock().await;
                    let uid = client.sender_id().to_string();
                    for command in crate::metadata::media_remote(
                        &uid,
                        &session.session_uuid,
                        &session.group_uuid,
                    ) {
                        let label = command
                            .as_dictionary()
                            .and_then(|v| v.get("type"))
                            .and_then(Value::as_string)
                            .unwrap_or("DeviceInfo");
                        let mut body = Vec::new();
                        command.to_writer_binary(&mut body)?;
                        let response = client
                            .request(
                                "POST",
                                Some("/command"),
                                Some("application/x-apple-binary-plist"),
                                &body,
                                &[],
                            )
                            .await?;
                        if let Err(error) = response.ok() {
                            eprintln!(
                                "{}: MediaRemote {label} rejected: {error:#}",
                                session.address
                            );
                            break;
                        }
                        eprintln!("{}: MediaRemote {label} accepted", session.address);
                    }
                }
            }
            Ok(())
        };
        tokio::select! {
            result = drain_during(input, publish) => result?,
            _ = &mut interrupt => return Ok(RunEnd::Stopped),
        }
    }
    let latency = sessions
        .iter()
        .filter_map(|s| s.reported_latency)
        .fold(requested, u32::max);
    ensure!(
        latency <= rate * 5,
        "receiver requested more than 5 seconds latency; refusing unbounded buffering"
    );
    eprintln!(
        "Requested {} ms; common schedule {} ms (includes any Audio-Latency reported by receivers). This is NOT a measured acoustic delay.",
        f64::from(requested) * 1000.0 / f64::from(rate),
        f64::from(latency) * 1000.0 / f64::from(rate)
    );
    // Do not start the shared clock until the FIRST input packet exists.
    let first = tokio::select! {
        _ = &mut interrupt => return Ok(RunEnd::Stopped),
        Some(error) = errors.recv() => return Err(anyhow!(error)),
        packet = input.packet(encoded) => packet?,
    };
    let Some(first) = first else {
        ensure!(
            !input.live,
            "Live capture ended without producing audio; check input diagnostics"
        );
        return Ok(RunEnd::Stopped);
    };
    let mut recognition = recognize.then(crate::recognition::Tap::start);
    let timeline = Timeline::with_rate(initial_rtp, rate);
    for session in sessions.iter_mut() {
        session.start(
            timeline.clone(),
            latency,
            recognition.as_ref().map(|tap| tap.updates.clone()),
        );
    }
    eprintln!(
        "control_event={}",
        serde_json::json!({"type":"playing","schedule_ms":f64::from(latency)*1000.0/f64::from(rate)})
    );
    let mut meter = crate::telemetry::Meter::default();
    let mut visualization = if encoded {
        None
    } else {
        crate::visualization::Tap::from_env()
    };
    let mut next = Some(first);
    let mut sent = 0u64;
    loop {
        let read_start = Instant::now();
        let packet = if let Some(first) = next.take() {
            Some(first)
        } else {
            tokio::select! {
                _ = &mut interrupt => return Ok(RunEnd::Stopped),
                Some(error) = errors.recv() => return Err(anyhow!(error)),
                packet = input.packet(encoded) => packet?,
            }
        };
        let read_us = read_start.elapsed().as_micros() as u64;
        metrics.max_read = metrics.max_read.max(read_us);
        let Some(mut pcm) = packet else {
            ensure!(
                !input.live,
                "Live capture ended unexpectedly; check input diagnostics"
            );
            break;
        };
        if !encoded {
            meter.observe(&pcm);
        }
        if let Some(tap) = visualization.as_mut() {
            tap.observe(&pcm);
        }
        if let Some(tap) = recognition.as_mut() {
            tap.observe(&pcm);
        }
        if silence
            .as_mut()
            .is_some_and(|detector| detector.observe(&pcm))
        {
            eprintln!("Silence timeout: releasing HomePods and returning to input monitoring.");
            return Ok(RunEnd::Silence);
        }
        let elapsed =
            Duration::from_nanos((u128::from(sent) * 1_000_000_000 / u128::from(rate)) as u64);
        let deadline = Instant::from_std(timeline.start + elapsed);
        // Record input stalls separately from waking late after the timer wait.
        let pre_wait_late_us = Instant::now()
            .saturating_duration_since(deadline)
            .as_micros() as u64;
        metrics.max_pre_wait_late = metrics.max_pre_wait_late.max(pre_wait_late_us);
        if pre_wait_late_us >= 100_000 {
            metrics.outlier(serde_json::json!({
                "packet": metrics.packets, "at_ms": timeline.start.elapsed().as_millis(),
                "stage": "before_timer_abort", "read_us": read_us,
                "pre_wait_late_us": pre_wait_late_us
            }));
            return Err(anyhow!(
                "input/sender fell more than 100 ms behind; stopping instead of accumulating latency"
            ));
        }
        tokio::select! {
            _ = &mut interrupt => return Ok(RunEnd::Stopped),
            Some(error) = errors.recv() => return Err(anyhow!(error)),
            _ = sleep_until(deadline) => {},
        }
        let late_us = Instant::now()
            .saturating_duration_since(deadline)
            .as_micros() as u64;
        let work_start = Instant::now();
        if !encoded {
            gain.apply(&mut pcm);
        }
        let seq = initial_seq.wrapping_add((sent / u64::from(frames)) as u16);
        let timestamp = initial_rtp.wrapping_add(sent as u32);
        let mut header = [0; 12];
        header[..2].copy_from_slice(&[0x80, if sent == 0 { 0xe0 } else { 0x60 }]);
        header[2..4].copy_from_slice(&seq.to_be_bytes());
        header[4..8].copy_from_slice(&timestamp.to_be_bytes());
        // One SSRC/sequence/timeline for both receivers; unique audio keys/nonces per session.
        header[8..12].copy_from_slice(&initial_rtp.to_be_bytes());
        for session in sessions.iter_mut() {
            session.send(&header, &pcm).await?;
        }
        let work_us = work_start.elapsed().as_micros() as u64;
        if late_us >= 5_000 || work_us >= 5_000 || read_us >= 5_000 {
            metrics.outlier(serde_json::json!({
                "packet": metrics.packets, "at_ms": timeline.start.elapsed().as_millis(),
                "stage": "sent", "read_us": read_us,
                "pre_wait_late_us": pre_wait_late_us, "wake_late_us": late_us,
                "work_us": work_us
            }));
        }
        metrics.record(late_us, work_us, pcm.len());
        sent += u64::from(frames);
    }
    eprintln!("Input ended; draining the scheduled receiver delay…");
    tokio::select! {
        _ = &mut interrupt => {},
        Some(error) = errors.recv() => return Err(anyhow!(error)),
        _ = tokio::time::sleep(Duration::from_secs_f64(f64::from(latency) / f64::from(rate) + 0.1)) => {},
    }
    eprintln!("Sent {sent} stereo frames to {} receivers.", sessions.len());
    Ok(RunEnd::Stopped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chacha20poly1305::{
        ChaCha20Poly1305, KeyInit, Nonce,
        aead::{Aead, Payload},
    };
    #[tokio::test]
    async fn pcm_partial_reads_survive_setup_completion() {
        use tokio::io::AsyncWriteExt;
        let (reader, mut writer) = tokio::io::duplex(PACKET_BYTES * 2);
        let expected: Vec<u8> = (0..PACKET_BYTES).map(|i| (i % 251) as u8).collect();
        let mut input = InputReader {
            reader: Box::new(reader),
            _capture: None,
            live: false,
            pending_pcm: vec![0; PACKET_BYTES],
            filled_pcm: 0,
        };
        writer.write_all(&expected[..3]).await.unwrap();
        drain_during(&mut input, async {
            tokio::time::sleep(Duration::from_millis(5)).await;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(input.filled_pcm, 3);
        writer.write_all(&expected[3..]).await.unwrap();
        assert_eq!(input.packet(false).await.unwrap().unwrap(), expected);
    }

    #[test]
    fn rtp_payload_has_authenticated_header_and_explicit_nonce() {
        let key = [1; 32];
        let header = [0x80, 0xe0, 0xff, 0xff, 0, 0, 0, 42, 0, 0, 0, 7];
        let packet = audio_packet(&mut Cipher::new(&key), &header, &[1, 2, 3, 4]).unwrap();
        assert_eq!(&packet[..12], &header);
        assert_eq!(&packet[packet.len() - 8..], &[0; 8]);
        let cipher = ChaCha20Poly1305::new((&key).into());
        let decrypted = cipher
            .decrypt(
                Nonce::from_slice(&[0; 12]),
                Payload {
                    msg: &packet[12..packet.len() - 8],
                    aad: &header[4..],
                },
            )
            .unwrap();
        assert_eq!(decrypted, [1, 2, 3, 4]);
    }
    #[tokio::test]
    async fn pcm_tail_is_padded_and_misaligned_input_rejected() {
        let bytes = [1, 2, 3, 4];
        let packet = read_packet(&mut bytes.as_slice()).await.unwrap().unwrap();
        assert_eq!(packet.len(), PACKET_BYTES);
        assert_eq!(&packet[..4], &bytes);
        assert!(packet[4..].iter().all(|b| *b == 0));
        assert!(read_packet(&mut [1, 2, 3].as_slice()).await.is_err());
    }

    #[tokio::test]
    async fn encoded_access_unit_framing_rejects_truncation_and_oversize() {
        let mut bytes = &[0, 3, 10, 20, 30][..];
        assert_eq!(
            read_source_packet(&mut bytes, true).await.unwrap(),
            Some(vec![10, 20, 30])
        );
        assert!(
            read_source_packet(&mut bytes, true)
                .await
                .unwrap()
                .is_none()
        );
        for data in [&[0][..], &[0, 3, 10][..], &[0, 0][..], &[0xff, 0xff][..]] {
            assert!(read_source_packet(&mut &data[..], true).await.is_err());
        }
    }
}
