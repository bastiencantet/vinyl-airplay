//! LAN controller. Serializes changes and owns exactly one unprivileged audio process.
use crate::receivers;
use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{Mutex, RwLock, oneshot},
};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_input")]
    input_device: String,
    enabled: bool,
    auto_connect: bool,
    targets: Vec<String>,
    latency_ms: u32,
    gain_db: i16,
    recognize: bool,
}
fn default_input() -> String {
    "default".into()
}
impl Default for Config {
    fn default() -> Self {
        Self {
            input_device: default_input(),
            enabled: false,
            auto_connect: true,
            targets: vec![],
            latency_ms: 250,
            gain_db: 0,
            recognize: false,
        }
    }
}
impl Config {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.input_device.is_empty()
                && self.input_device.len() <= 2048
                && !self.input_device.contains('\0'),
            "Entrée audio invalide."
        );
        ensure!(
            self.targets.len() <= 8 && (!self.enabled || !self.targets.is_empty()),
            "Sélectionne entre une et huit enceintes."
        );
        ensure!(
            self.targets
                .iter()
                .all(|t| matches!(t.as_str(), "salon" | "chambre") || receivers::valid_id(t)),
            "Enceinte inconnue."
        );
        ensure!(
            self.targets.iter().collect::<HashSet<_>>().len() == self.targets.len(),
            "Enceinte en double."
        );
        ensure!(
            (70..=2000).contains(&self.latency_ms),
            "Le délai demandé doit être compris entre 70 et 2000 ms."
        );
        ensure!(
            (-12..=12).contains(&self.gain_db),
            "Le gain doit être compris entre −12 et +12 dB."
        );
        Ok(())
    }
}
#[derive(Serialize)]
struct Status {
    phase: String,
    rms_dbfs: Option<f64>,
    peak_dbfs: Option<f64>,
    meter_at: u64,
    channels_rms_dbfs: Option<[f64; 2]>,
    channels_peak_dbfs: Option<[f64; 2]>,
    spectrum_dbfs: Option<[f64; 12]>,
    schedule_ms: Option<f64>,
    track: Value,
    error: Option<String>,
    logs: VecDeque<String>,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            phase: "stopped".into(),
            rms_dbfs: None,
            peak_dbfs: None,
            meter_at: 0,
            channels_rms_dbfs: None,
            channels_peak_dbfs: None,
            spectrum_dbfs: None,
            schedule_ms: None,
            track: json!(null),
            error: None,
            logs: VecDeque::new(),
        }
    }
}
struct Manager {
    config: Config,
    addresses: BTreeMap<String, String>,
    child: Option<Child>,
    reader: Option<tokio::task::JoinHandle<()>>,
}
struct App {
    manager: Mutex<Manager>,
    status: Arc<RwLock<Status>>,
    token: String,
    root: PathBuf,
    config_path: PathBuf,
    receivers: receivers::Catalog,
    control_path: PathBuf,
    sender: PathBuf,
    legacy_input: Option<String>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
async fn auth(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    let supplied = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let expected = format!("Bearer {}", app.token);
    if !bool::from(supplied.as_bytes().ct_eq(expected.as_bytes())) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if req.method() != axum::http::Method::GET
        && req
            .headers()
            .get("x-vinyl-control")
            .and_then(|v| v.to_str().ok())
            != Some("1")
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(req).await
}
async fn state(State(app): State<Arc<App>>) -> Json<Value> {
    let mut manager = app.manager.lock().await;
    refresh(&mut manager, &app.status).await;
    let volumes = sender_control(&app, json!({"op":"status"}), Duration::from_millis(500))
        .await
        .ok()
        .and_then(|v| v.get("receivers").cloned())
        .unwrap_or_else(|| json!([]));
    Json(
        json!({"config":manager.config,"running":manager.child.is_some(),"status":*app.status.read().await,"volumes":volumes,"server_time":now(),"devices":app.receivers.list(),"active_targets":manager.addresses,"legacy_input":app.legacy_input}),
    )
}
async fn meter(State(app): State<Arc<App>>) -> Json<Value> {
    let s = app.status.read().await;
    Json(
        json!({"server_time":now(),"meter_at":s.meter_at,"rms_dbfs":s.rms_dbfs,"peak_dbfs":s.peak_dbfs,"channels_rms_dbfs":s.channels_rms_dbfs,"channels_peak_dbfs":s.channels_peak_dbfs,"spectrum_dbfs":s.spectrum_dbfs}),
    )
}
async fn refresh(manager: &mut Manager, status: &Arc<RwLock<Status>>) {
    #[cfg(unix)]
    let id = manager.child.as_ref().and_then(Child::id);
    if let Some(child) = manager.child.as_mut()
        && let Ok(Some(exit)) = child.try_wait()
    {
        #[cfg(unix)]
        if let Some(id) = id {
            signal_group(id, "-KILL").await;
        }
        let mut s = status.write().await;
        s.phase = "error".into();
        s.error = Some(format!(
            "Le programme audio s’est arrêté ({exit}). Relance la diffusion."
        ));
        manager.child = None;
    }
}
#[cfg(unix)]
async fn signal_group(id: u32, signal: &str) {
    let _ = Command::new("/bin/kill")
        .args([signal, "--", &format!("-{id}")])
        .status()
        .await;
}
async fn stop(manager: &mut Manager, app: &App) -> Result<()> {
    if let Some(mut child) = manager.child.take() {
        #[cfg(unix)]
        let id = child.id();
        let _ = sender_control(app, json!({"op":"shutdown"}), Duration::from_millis(500)).await;
        #[cfg(unix)]
        if let Some(id) = id {
            signal_group(id, "-INT").await;
        }
        let finished = tokio::time::timeout(Duration::from_secs(8), child.wait()).await;
        // Also reap descendants after a sender crash or forced stop: never leave arecord behind.
        #[cfg(unix)]
        if let Some(id) = id {
            signal_group(id, "-KILL").await;
        }
        if finished.is_err() {
            child.kill().await?;
        }
    }
    if let Some(reader) = manager.reader.take() {
        let _ = tokio::time::timeout(Duration::from_secs(1), reader).await;
    }
    Ok(())
}
async fn start(app: &App, manager: &mut Manager) -> Result<()> {
    let c = &manager.config;
    *app.status.write().await = Status {
        phase: if c.enabled { "starting" } else { "stopped" }.into(),
        ..Status::default()
    };
    if !c.enabled {
        return Ok(());
    }
    let (input_flag, input) = match &app.legacy_input {
        Some(device) => ("--alsa-device", device.as_str()),
        None => ("--input-device", c.input_device.as_str()),
    };
    let sender = &app.sender;
    let mut cmd = Command::new(sender);
    cmd.arg("stream").args([
        input_flag,
        input,
        "--latency-ms",
        &c.latency_ms.to_string(),
        "--gain-db",
        &c.gain_db.to_string(),
    ]);
    let addresses: BTreeMap<_, _> = c
        .targets
        .iter()
        .map(|id| Ok((id.clone(), app.receivers.resolve(id)?)))
        .collect::<Result<_>>()?;
    ensure!(
        addresses.values().collect::<HashSet<_>>().len() == addresses.len(),
        "Enceinte en double."
    );
    for address in addresses.values() {
        cmd.args(["--target", address]);
    }
    manager.addresses = addresses;
    if c.auto_connect {
        cmd.arg("--auto-connect");
    }
    if c.recognize {
        cmd.arg("--recognize");
    }
    cmd.env(
        "PATH",
        std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()),
    )
    .env(
        "VINYL_CONTROL_ARTWORK",
        app.root.join("control/artwork.jpg"),
    )
    .env("VINYL_CONTROL_SOCKET", &app.control_path)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    #[cfg(unix)]
    cmd.as_std_mut().process_group(0);
    #[cfg(windows)]
    cmd.creation_flags(0x00000200); // CREATE_NEW_PROCESS_GROUP: controller Ctrl-Break must not kill audio before TEARDOWN.
    let mut child = cmd.spawn().context("Démarrer le programme audio")?;
    let stderr = child.stderr.take().context("stderr")?;
    let status = app.status.clone();
    let (capture_tx, capture_rx) = oneshot::channel();
    let log_path = app.root.join("control/audio.log");
    manager.reader = Some(tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut capture_tx = Some(capture_tx);
        let mut file = std::fs::File::create(&log_path).ok();
        let mut logged_bytes = 0;
        while let Ok(Some(line)) = lines.next_line().await {
            // Events and diagnostics are bounded; no raw audio or credentials are logged.
            if (line.starts_with("Starting ALSA capture on ")
                || line.starts_with("Starting native capture on "))
                && let Some(tx) = capture_tx.take()
            {
                let _ = tx.send(());
            }
            if line.len() > 16384 {
                continue;
            }
            if logged_bytes > 2 * 1024 * 1024 {
                file.take();
                let _ = std::fs::rename(&log_path, log_path.with_extension("log.1"));
                file = std::fs::File::create(&log_path).ok();
                logged_bytes = 0;
            }
            logged_bytes += line.len() + 1;
            if let Some(file) = file.as_mut() {
                use std::io::Write;
                let _ = writeln!(file, "{line}");
            }
            let mut s = status.write().await;
            if let Some(event) = line
                .strip_prefix("control_event=")
                .and_then(|v| serde_json::from_str::<Value>(v).ok())
            {
                match event["type"].as_str() {
                    Some("meter") => {
                        s.rms_dbfs = event["rms_dbfs"].as_f64();
                        s.peak_dbfs = event["peak_dbfs"].as_f64();
                        s.meter_at = now();
                        s.channels_rms_dbfs =
                            serde_json::from_value(event["channels_rms_dbfs"].clone()).ok();
                        s.channels_peak_dbfs =
                            serde_json::from_value(event["channels_peak_dbfs"].clone()).ok();
                        s.spectrum_dbfs =
                            serde_json::from_value(event["spectrum_dbfs"].clone()).ok();
                    }
                    Some("playing") => {
                        s.phase = "playing".into();
                        s.schedule_ms = event["schedule_ms"].as_f64();
                        s.error = None;
                    }
                    Some("track") => s.track = event,
                    _ => {}
                }
            } else {
                if line.starts_with("Monitoring input:") {
                    s.phase = "monitoring".into();
                    s.track = Value::Null;
                    s.schedule_ms = None;
                }
                if line.starts_with("Input activity detected;") {
                    s.phase = "connecting".into();
                }
                if line.starts_with("Playback failed:") || line.starts_with("Error:") {
                    s.phase = "error".into();
                    s.error = Some(line.clone());
                }
                if line.starts_with("Silence timeout:") {
                    s.phase = "disconnecting".into();
                }
                if s.logs.len() == 12 {
                    s.logs.pop_front();
                }
                s.logs.push_back(line);
            }
        }
    }));
    manager.child = Some(child);
    tokio::time::timeout(Duration::from_secs(12), capture_rx)
        .await
        .context("L’ouverture de l’entrée audio a pris trop de temps")?
        .context("Le programme audio s’est arrêté avant la capture")?;
    refresh(manager, &app.status).await;
    ensure!(
        manager.child.is_some(),
        "Le programme audio n’a pas pu démarrer."
    );
    Ok(())
}
async fn apply(State(app): State<Arc<App>>, Json(config): Json<Config>) -> Response {
    if let Err(e) = config.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":e.to_string()})),
        )
            .into_response();
    }
    let mut m = app.manager.lock().await;
    refresh(&mut m, &app.status).await;
    if m.config == config && (!config.enabled || m.child.is_some()) {
        return Json(json!({"ok":true})).into_response();
    }
    if let Some(id) = config
        .targets
        .iter()
        .find(|id| app.receivers.find(id).is_none())
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":format!("Appareil AirPlay non détecté : {id}.")})),
        )
            .into_response();
    }
    let old = m.config.clone();
    let result: Result<()> = async {
        stop(&mut m, &app).await?;
        m.config = config;
        start(&app, &mut m).await?;
        let tmp = app.config_path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&m.config)?)?;
        std::fs::rename(tmp, &app.config_path)?;
        Ok(())
    }
    .await;
    if let Err(error) = result {
        let message = format!("{error:#}");
        let _ = stop(&mut m, &app).await;
        m.config = old;
        let rollback = start(&app, &mut m).await.err().map(|e| format!("{e:#}"));
        app.status.write().await.error = Some(message.clone());
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":message,"rollback_error":rollback})),
        )
            .into_response();
    }
    Json(json!({"ok":true})).into_response()
}
async fn sender_control(app: &App, request: Value, deadline: Duration) -> Result<Value> {
    tokio::time::timeout(deadline, async {
        let mut socket = crate::ipc::connect(&app.control_path).await?;
        let mut bytes = serde_json::to_vec(&request)?;
        bytes.push(b'\n');
        socket.write_all(&bytes).await?;
        let mut reader = BufReader::new(socket).take(16385);
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        ensure!(
            line.len() <= 16384 && line.ends_with('\n'),
            "Invalid bounded sender response"
        );
        Ok(serde_json::from_str(&line)?)
    })
    .await?
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VolumeRequest {
    target: String,
    percent: u8,
}
async fn set_volume(State(app): State<Arc<App>>, Json(request): Json<VolumeRequest>) -> Response {
    if app.receivers.find(&request.target).is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"Appareil AirPlay inconnu."})),
        )
            .into_response();
    }
    if request.percent > 100 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"Volume hors limites (0..100)."})),
        )
            .into_response();
    }
    // Serialize against process replacement, but never touch the PCM loop or config.
    let manager = app.manager.lock().await;
    let Some(target) = manager.addresses.get(&request.target) else {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error":"Cette enceinte ne reçoit pas le vinyle."})),
        )
            .into_response();
    };
    match sender_control(
        &app,
        json!({"op":"set","target":target,"percent":request.percent}),
        Duration::from_secs(14),
    )
    .await
    {
        Ok(value) if value["ok"] == true => Json(value).into_response(),
        Ok(value) => (StatusCode::CONFLICT, Json(value)).into_response(),
        Err(_) => (
            StatusCode::CONFLICT,
            Json(
                json!({"error":"Volume disponible quand le HomePod est connecté à la diffusion."}),
            ),
        )
            .into_response(),
    }
}
async fn artwork(State(app): State<Arc<App>>) -> Response {
    if app.status.read().await.track["artwork"] != true {
        return StatusCode::NOT_FOUND.into_response();
    }
    match std::fs::read(app.root.join("control/artwork.jpg")) {
        Ok(bytes) if bytes.len() <= 256 * 1024 => (
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}
/// Shared controller: the web adapter and desktop adapter own the same supervisor.
#[derive(Clone)]
pub struct Controller(Arc<App>);
impl Controller {
    pub async fn open(
        root: PathBuf,
        sender: PathBuf,
        token: String,
        legacy_input: Option<String>,
        input_override: Option<String>,
    ) -> Result<Self> {
        ensure!(token.len() >= 32, "token too short");
        std::fs::create_dir_all(root.join("control"))?;
        let config_path = root.join("control/config.json");
        let mut config: Config = if Path::new(&config_path).exists() {
            serde_json::from_slice(&std::fs::read(&config_path)?)?
        } else {
            Config::default()
        };
        if let Some(input) = input_override {
            config.input_device = input;
        }
        config.validate()?;
        let receivers = receivers::Catalog::start(root.join("control/receivers.json"))?;
        // Allow the first Bonjour responses before resolving the saved selection.
        for _ in 0..50 {
            if config.targets.iter().all(|id| receivers.find(id).is_some()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        for id in &mut config.targets {
            if let Some(receiver) = receivers.find(id) {
                *id = receiver.id;
            }
        }
        // Persist legacy room-name migration; no IP is embedded in the new config.
        let tmp = config_path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&config)?)?;
        std::fs::rename(tmp, &config_path)?;
        let app = Arc::new(App {
            manager: Mutex::new(Manager {
                config,
                addresses: BTreeMap::new(),
                child: None,
                reader: None,
            }),
            status: Arc::new(RwLock::new(Status::default())),
            token,
            control_path: crate::ipc::endpoint(&root),
            root,
            sender,
            legacy_input,
            config_path,
            receivers,
        });
        {
            let mut m = app.manager.lock().await;
            if let Err(error) = start(&app, &mut m).await {
                let _ = stop(&mut m, &app).await;
                let mut status = app.status.write().await;
                status.phase = "error".into();
                status.error = Some(format!("{error:#}"));
            }
        }
        Ok(Self(app))
    }
    pub async fn request(&self, operation: &str, payload: Option<Value>) -> Result<Value> {
        let app = self.0.clone();
        let response = match operation {
            "state" => return Ok(state(State(app)).await.0),
            "meter" => return Ok(meter(State(app)).await.0),
            "inputs" => return tokio::task::spawn_blocking(crate::audio::devices).await?,
            "config" => {
                apply(
                    State(app),
                    Json(serde_json::from_value(payload.context("missing config")?)?),
                )
                .await
            }
            "volume" => {
                set_volume(
                    State(app),
                    Json(serde_json::from_value(payload.context("missing volume")?)?),
                )
                .await
            }
            _ => anyhow::bail!("unsupported controller operation"),
        };
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024).await?;
        let value: Value = serde_json::from_slice(&body)?;
        ensure!(
            status.is_success(),
            "{}",
            value["error"]
                .as_str()
                .unwrap_or("Controller request failed")
        );
        Ok(value)
    }
    pub async fn artwork(&self) -> Result<Vec<u8>> {
        ensure!(
            self.0.status.read().await.track["artwork"] == true,
            "no artwork"
        );
        let bytes = std::fs::read(self.0.root.join("control/artwork.jpg"))?;
        ensure!(bytes.len() <= 256 * 1024, "artwork exceeds limit");
        Ok(bytes)
    }
    pub async fn stop(&self) -> Result<()> {
        stop(&mut *self.0.manager.lock().await, &self.0).await
    }
    pub async fn disable(&self) -> Result<()> {
        let config = {
            let manager = self.0.manager.lock().await;
            let mut config = manager.config.clone();
            config.enabled = false;
            config
        };
        self.request("config", Some(serde_json::to_value(config)?))
            .await?;
        Ok(())
    }
    pub(crate) fn routes(&self) -> Router {
        let app = self.0.clone();
        Router::new()
            .route("/api/state", get(state))
            .route("/api/meter", get(meter))
            .route("/api/inputs", get(inputs))
            .route("/api/config", post(apply))
            .route("/api/artwork", get(artwork))
            .route("/api/volume", post(set_volume))
            .layer(DefaultBodyLimit::max(4096))
            .with_state(app)
    }
    pub async fn serve(&self, bind: &str) -> Result<()> {
        let app = self.0.clone();
        let router = self
            .routes()
            .layer(middleware::from_fn_with_state(app, auth));
        let listener = tokio::net::TcpListener::bind(bind).await?;
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                #[cfg(unix)]
                let _ = tokio::signal::ctrl_c().await;
                #[cfg(windows)]
                {
                    let mut breaks =
                        tokio::signal::windows::ctrl_break().expect("register Ctrl-Break");
                    tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = breaks.recv() => {} }
                }
            })
            .await?;
        self.stop().await
    }
}
async fn inputs() -> Response {
    match tokio::task::spawn_blocking(crate::audio::devices).await {
        Ok(Ok(value)) => Json(value).into_response(),
        result => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":format!("Input discovery unavailable: {result:?}")})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_reject_arbitrary_receivers_and_dangerous_ranges() {
        assert!(Config::default().validate().is_ok());
        for c in [
            Config {
                targets: vec!["http://localhost".into()],
                ..Config::default()
            },
            Config {
                targets: vec!["salon".into(), "salon".into()],
                ..Config::default()
            },
            Config {
                gain_db: 13,
                ..Config::default()
            },
            Config {
                latency_ms: 0,
                ..Config::default()
            },
        ] {
            assert!(c.validate().is_err());
        }
        assert!(
            serde_json::from_value::<Config>(
                json!({"enabled":true,"targets":["salon"],"shell":"foo"})
            )
            .is_err()
        );
    }
}
