//! Native launcher for the embedded browser interface and shared controller.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use std::{
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
};
#[derive(Parser)]
#[command(version, about = "Local Rust web interface for Vinyl AirPlay")]
struct Args {
    /// Native audio input ID; uses the saved selection or system default if omitted.
    #[arg(long, conflicts_with = "alsa_device")]
    input_device: Option<String>,
    /// Legacy Linux arecord input; overrides native input selection.
    #[arg(long)]
    alsa_device: Option<String>,
    /// Specific private IPv4 or loopback. All-interface/public binds are rejected.
    #[arg(long, default_value = "127.0.0.1")]
    listen: Ipv4Addr,
    #[arg(long, default_value_t = 8787, value_parser = clap::value_parser!(u16).range(1024..))]
    port: u16,
    /// State directory; uses the OS user state directory by default.
    #[arg(long)]
    state: Option<PathBuf>,
    /// Sender executable; defaults to vinyl-airplay beside this launcher.
    #[arg(long)]
    sender: Option<PathBuf>,
}
fn default_state() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        Ok(PathBuf::from(
            std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is unavailable; use --state")?,
        )
        .join("vinyl-airplay-web"))
    }
    #[cfg(not(windows))]
    {
        let home =
            PathBuf::from(std::env::var_os("HOME").context("HOME is unavailable; use --state")?);
        #[cfg(target_os = "macos")]
        {
            Ok(home.join("Library/Application Support/vinyl-airplay-web"))
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(std::env::var_os("XDG_STATE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/state"))
                .join("vinyl-airplay"))
        }
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let bind = SocketAddrV4::new(args.listen, args.port);
    vinyl_airplay::web::validate_bind(bind)?;
    let sender = args.sender.unwrap_or(
        std::env::current_exe()?
            .with_file_name(format!("vinyl-airplay{}", std::env::consts::EXE_SUFFIX)),
    );
    ensure!(
        sender.is_file(),
        "sender executable is missing; build/install vinyl-airplay beside vinyl-web or use --sender"
    );
    let root = args.state.map_or_else(default_state, Ok)?;
    let mut directory = std::fs::DirBuilder::new();
    directory.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory.create(&root)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("web.lock"))?;
    lock.try_lock()
        .context("another web launcher owns this state directory")?;
    // Reserve the HTTP port before restoring a saved playback session.
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let controller = vinyl_airplay::control::Controller::open(
        root,
        sender,
        token,
        args.alsa_device,
        args.input_device,
    )
    .await?;
    eprintln!("Open http://{bind}/ — native Rust web interface.");
    vinyl_airplay::web::serve(controller, listener).await
}
