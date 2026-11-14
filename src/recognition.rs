//! Optional Shazam fingerprint recognition. Bounded tap; never awaits in the audio loop.
use crate::{activity::rms_dbfs, metadata::Track};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::{mpsc, watch},
    task::JoinHandle,
};

const WINDOW: usize = 12 * 44100 * 4;
const PERIOD: usize = 30 * 44100 * 4;
const MAX_JSON: u64 = 512 * 1024;
const MAX_ART: u64 = 256 * 1024;

pub struct Tap {
    samples: Vec<u8>,
    position: usize,
    jobs: mpsc::Sender<(Instant, Vec<u8>)>,
    task: JoinHandle<()>,
    pub updates: watch::Receiver<Track>,
}

impl Tap {
    pub fn start() -> Self {
        let (jobs, mut rx) = mpsc::channel::<(Instant, Vec<u8>)>(1);
        let (tx, updates) = watch::channel(Track::default());
        let task = tokio::spawn(async move {
            let mut last_match = None;
            let mut backoff = Duration::from_secs(30);
            let mut next_attempt = Instant::now();
            let mut expiry = tokio::time::interval(Duration::from_secs(10));
            loop {
                tokio::select! {
                    job = rx.recv() => {
                        let Some((captured, pcm)) = job else { break; };
                        if captured.elapsed() > Duration::from_secs(20) { continue; }
                        if let Some(wait) = next_attempt.checked_duration_since(Instant::now()) {
                            if wait > Duration::from_secs(1) { continue; }
                            tokio::time::sleep(wait).await;
                        }
                        let Ok(Some(pcm)) = tokio::task::spawn_blocking(move || (rms_dbfs(&pcm) >= -55.0).then_some(pcm)).await else { continue; };
                        match identify(&pcm).await {
                            Ok(Some(track)) => {
                                last_match = Some(Instant::now());
                                backoff = Duration::from_secs(30);
                                next_attempt = captured + backoff;
                                let previous = tx.borrow().clone();
                                if previous == track { continue; }
                                eprintln!("Recognized: {} — {} / {}", track.artist, track.title, track.album);
                                if let Some(path) = std::env::var_os("VINYL_CONTROL_ARTWORK") {
                                    if let Some(bytes) = &track.artwork {
                                        let _ = std::fs::write(path, bytes);
                                    } else { let _ = std::fs::remove_file(path); }
                                }
                                eprintln!("control_event={}", json!({"type":"track", "title":track.title,"artist":track.artist,"album":track.album,"key":track.key,"artwork":track.artwork.is_some()}));
                                tx.send_replace(track);
                            }
                            Ok(None) => { backoff = Duration::from_secs(30); next_attempt = captured + backoff; }
                            Err(error) => {
                                eprintln!("Music recognition unavailable: {error:#}; retry after {} s (audio continues).", backoff.as_secs());
                                next_attempt = Instant::now() + backoff;
                                backoff = (backoff * 2).min(Duration::from_secs(300));
                            }
                        }
                    }
                    _ = expiry.tick() => {
                        if last_match.is_some_and(|time: Instant| time.elapsed() > Duration::from_secs(90)) {
                            eprintln!("control_event={}", json!({"type":"track","title":"Vinyle","artist":"Entrée audio","album":"Entrée ligne","key":"","artwork":false}));
                            tx.send_replace(Track::default());
                            last_match = None;
                        }
                    }
                }
            }
        });
        Self {
            samples: Vec::with_capacity(WINDOW),
            position: 0,
            jobs,
            task,
            updates,
        }
    }

    pub fn observe(&mut self, pcm: &[u8]) {
        if self.position < WINDOW {
            self.samples
                .extend_from_slice(&pcm[..pcm.len().min(WINDOW - self.position)]);
        }
        let previous = self.position;
        self.position += pcm.len();
        if previous < WINDOW && self.position >= WINDOW {
            let samples = std::mem::replace(&mut self.samples, Vec::with_capacity(WINDOW));
            let _ = self.jobs.try_send((Instant::now(), samples));
        }
        if self.position >= PERIOD {
            self.position = 0;
        }
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn fingerprint_program() -> Result<PathBuf> {
    Ok(std::env::var_os("VINYL_SHAZAM_FINGERPRINT")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_exe()?.with_file_name(format!(
            "vinyl-shazam-fingerprint{}",
            std::env::consts::EXE_SUFFIX
        ))))
}

async fn process(mut command: Command, input: Option<&[u8]>, limit: u64) -> Result<Vec<u8>> {
    command
        .kill_on_drop(true)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .context("start recognition helper or curl")?;
    tokio::time::timeout(Duration::from_secs(20), async {
        if let Some(input) = input {
            let mut stdin = child.stdin.take().context("helper stdin")?;
            stdin.write_all(input).await?;
            drop(stdin);
        }
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .context("helper stdout")?
            .take(limit + 1)
            .read_to_end(&mut output)
            .await?;
        ensure!(
            output.len() as u64 <= limit,
            "recognition response exceeds limit"
        );
        ensure!(
            child.wait().await?.success(),
            "recognition helper or HTTP request failed"
        );
        Ok(output)
    })
    .await
    .context("recognition timed out")?
}

async fn http(url: &str, body: Option<&[u8]>, limit: u64) -> Result<Vec<u8>> {
    let mut command = Command::new("curl");
    command.args([
        "--silent",
        "--show-error",
        "--location",
        "--max-redirs",
        "2",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        "5",
        "--max-time",
        "15",
        "--max-filesize",
        &limit.to_string(),
        "--write-out",
        "\n%{http_code}",
        "--user-agent",
        "Shazam/14.1.0",
    ]);
    if body.is_some() {
        command.args([
            "--header",
            "Content-Type: application/json",
            "--data-binary",
            "@-",
        ]);
    }
    command.arg(url);
    let mut bytes = process(command, body, limit + 4).await?;
    ensure!(bytes.len() >= 4, "missing HTTP status");
    let status = String::from_utf8_lossy(&bytes[bytes.len() - 3..]).to_string();
    ensure!(status == "200", "recognition HTTP {status}");
    bytes.truncate(bytes.len() - 4);
    Ok(bytes)
}

pub async fn identify(pcm: &[u8]) -> Result<Option<Track>> {
    #[cfg(unix)]
    let command = {
        let mut cmd = Command::new("nice");
        cmd.args(["-n", "10"]).arg(fingerprint_program()?);
        cmd
    };
    #[cfg(windows)]
    let command = Command::new(fingerprint_program()?);
    let signature: Value = serde_json::from_slice(&process(command, Some(pcm), MAX_JSON).await?)?;
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let body = serde_json::to_vec(
        &json!({"signature": {"uri": signature["uri"], "samplems": signature["samplems"], "timestamp": timestamp}, "timestamp": timestamp, "timezone": "Europe/Paris"}),
    )?;
    let url = format!(
        "https://amp.shazam.com/discovery/v5/en/US/android/-/tag/{}/{}?sync=true&webv3=true&sampling=true&shazamapiversion=v3&sharehub=true&video=v3",
        uuid::Uuid::new_v4().to_string().to_uppercase(),
        uuid::Uuid::new_v4()
    );
    let result: Value = serde_json::from_slice(&http(&url, Some(&body), MAX_JSON).await?)?;
    let Some((mut track, cover)) = parse_track(&result)? else {
        return Ok(None);
    };
    if let Some(url) = cover
        && let Ok(bytes) = http(&url, None, MAX_ART).await
        && bytes.starts_with(&[0xff, 0xd8, 0xff])
    {
        track.artwork = Some(bytes);
    }
    Ok(Some(track))
}

fn parse_track(result: &Value) -> Result<Option<(Track, Option<String>)>> {
    let Some(track) = result.get("track") else {
        return Ok(None);
    };
    let text = |key| {
        track
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.len() <= 1024)
            .map(str::to_owned)
    };
    let title = text("title").context("recognition returned no title")?;
    let artist = text("subtitle").unwrap_or_default();
    let key = text("key").context("recognition returned no track key")?;
    let album = track
        .get("sections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|section| {
            section
                .get("metadata")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .find(|entry| {
            entry
                .get("title")
                .and_then(Value::as_str)
                .is_some_and(|v| v.eq_ignore_ascii_case("Album"))
        })
        .and_then(|entry| entry.get("text"))
        .and_then(Value::as_str)
        .filter(|s| s.len() <= 1024)
        .unwrap_or("")
        .to_owned();
    let cover = track
        .pointer("/images/coverart")
        .and_then(Value::as_str)
        .filter(|url| safe_artwork_url(url))
        .map(str::to_owned);
    Ok(Some((
        Track {
            key,
            title,
            artist,
            album,
            artwork: None,
        },
        cover,
    )))
}

fn safe_artwork_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    !host.contains(['@', ':']) && (host.ends_with(".mzstatic.com") || host.ends_with(".shazam.com"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn service_metadata_and_unknown_results() {
        let (track, _) = parse_track(&json!({"track": {"key":"123", "title":"Été", "subtitle":"Artiste", "sections":[{"metadata":[{"title":"Album", "text":"Disque"}]}]}})).unwrap().unwrap();
        assert_eq!(
            (
                track.title.as_str(),
                track.artist.as_str(),
                track.album.as_str()
            ),
            ("Été", "Artiste", "Disque")
        );
        assert!(parse_track(&json!({"matches":[]})).unwrap().is_none());
        assert!(parse_track(&json!({"track":{}})).is_err());
        assert!(safe_artwork_url("https://is1-ssl.mzstatic.com/image.jpg"));
        assert!(!safe_artwork_url(
            "https://is1-ssl.mzstatic.com@127.0.0.1/x"
        ));
        assert!(!safe_artwork_url("http://is1-ssl.mzstatic.com/x"));
    }
}
