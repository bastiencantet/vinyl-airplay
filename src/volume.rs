//! Receiver volume via the existing encrypted RTSP connection. Audio PCM is untouched.
use crate::rtsp::Client;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex, OnceLock},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{Mutex, Semaphore},
    task::JoinHandle,
};

pub type Level = Arc<StdMutex<Option<f64>>>;
#[derive(Clone)]
struct Receiver {
    client: Arc<Mutex<Client>>,
    level: Level,
}
fn registry() -> &'static StdMutex<HashMap<SocketAddr, Receiver>> {
    static REGISTRY: OnceLock<StdMutex<HashMap<SocketAddr, Receiver>>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}
pub fn valid_db(db: f64) -> bool {
    db.is_finite() && (-144.0..=0.0).contains(&db)
}
pub fn percent(db: f64) -> f64 {
    (100.0 * (db / 30.0 + 1.0)).clamp(0.0, 100.0)
}
pub fn from_percent(percent: u8) -> Result<f64> {
    ensure!(percent <= 100, "Volume hors limites (0..100).");
    Ok(if percent == 0 {
        -144.0
    } else {
        f64::from(percent) * 0.3 - 30.0
    })
}
pub fn register(target: SocketAddr, client: Arc<Mutex<Client>>, level: Level) {
    registry()
        .lock()
        .expect("volume registry")
        .insert(target, Receiver { client, level });
}
pub fn unregister(target: SocketAddr) {
    registry().lock().expect("volume registry").remove(&target);
}
pub fn observed(target: SocketAddr, level: &Level, db: f64) {
    if !valid_db(db) {
        return;
    }
    *level.lock().expect("volume level") = Some(db);
    eprintln!(
        "control_event={}",
        json!({"type":"volume","target":target,"db":db,"percent":percent(db)})
    );
}
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Status,
    Shutdown,
    Set { target: SocketAddr, percent: u8 },
}
async fn handle(request: Request) -> Result<Value> {
    match request {
        Request::Shutdown => {
            crate::shutdown::request();
            Ok(json!({"ok":true}))
        }
        Request::Status => {
            let entries = registry().lock().expect("volume registry");
            let receivers: Vec<_> = entries
                .iter()
                .map(|(target, entry)| {
                    let db = *entry.level.lock().expect("volume level");
                    json!({"target":target,"db":db,"percent":db.map(percent)})
                })
                .collect();
            Ok(json!({"ok":true,"receivers":receivers}))
        }
        Request::Set {
            target,
            percent: requested,
        } => {
            let db = from_percent(requested)?;
            let receiver = registry()
                .lock()
                .expect("volume registry")
                .get(&target)
                .cloned()
                .context("Cette enceinte n’est pas connectée au flux.")?;
            let body = format!("volume: {db:.6}\r\n");
            receiver
                .client
                .lock()
                .await
                .request(
                    "SET_PARAMETER",
                    None,
                    Some("text/parameters"),
                    body.as_bytes(),
                    &[],
                )
                .await?
                .ok()?;
            observed(target, &receiver.level, db);
            Ok(json!({"ok":true,"target":target,"db":db,"percent":requested}))
        }
    }
}
pub struct Server {
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub fn start() -> Result<Option<Server>> {
    let Some(path) = std::env::var_os("VINYL_CONTROL_SOCKET").map(PathBuf::from) else {
        return Ok(None);
    };
    let mut listener = crate::ipc::Listener::bind(&path)?;
    let task = tokio::spawn(async move {
        let limit = Arc::new(Semaphore::new(4));
        while let Ok(socket) = listener.accept().await {
            let Ok(permit) = limit.clone().try_acquire_owned() else {
                continue;
            };
            tokio::spawn(async move {
                let _permit = permit;
                let (reader, mut writer) = tokio::io::split(socket);
                let mut reader = BufReader::new(reader);
                let result: Result<Value> = async {
                    let mut line = String::new();
                    // Limit the request before allocating an arbitrarily large line.
                    use tokio::io::AsyncReadExt;
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        (&mut reader).take(2049).read_line(&mut line),
                    )
                    .await??;
                    ensure!(
                        line.len() <= 2048 && line.ends_with('\n'),
                        "Invalid bounded control request"
                    );
                    let request = serde_json::from_str(&line)?;
                    tokio::time::timeout(Duration::from_secs(12), handle(request)).await?
                }
                .await;
                let response =
                    result.unwrap_or_else(|e| json!({"ok":false,"error":format!("{e:#}")}));
                if let Ok(mut bytes) = serde_json::to_vec(&response) {
                    bytes.push(b'\n');
                    let _ = writer.write_all(&bytes).await;
                    #[cfg(windows)]
                    {
                        // Closing a named pipe before its client consumes the reply can
                        // discard that reply. The bounded client closes after reading.
                        use tokio::io::AsyncReadExt;
                        let _ = tokio::time::timeout(Duration::from_secs(2), reader.read(&mut [0]))
                            .await;
                    }
                }
            });
        }
    });
    Ok(Some(Server { task }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_and_mute() {
        assert_eq!(from_percent(0).unwrap(), -144.0);
        assert_eq!(from_percent(100).unwrap(), 0.0);
        assert!((from_percent(50).unwrap() + 15.0).abs() < 1e-8);
        assert!(from_percent(101).is_err());
        assert!(!valid_db(f64::NAN));
        assert!(!valid_db(0.1));
        assert_eq!(percent(-144.0), 0.0);
        assert_eq!(percent(-15.0), 50.0);
    }
}
