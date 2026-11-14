//! Authenticated HTTP adapter for the shared controller.
use anyhow::{Context, Result};
use std::path::PathBuf;
#[tokio::main]
async fn main() -> Result<()> {
    let root = PathBuf::from(std::env::var("VINYL_ROOT").context("VINYL_ROOT missing")?);
    let token = std::fs::read_to_string(std::env::var("VINYL_TOKEN_FILE")?)?
        .trim()
        .to_owned();
    let sender = std::env::var_os("VINYL_SENDER_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(format!("bin/vinyl-airplay{}", std::env::consts::EXE_SUFFIX)));
    let controller = vinyl_airplay::control::Controller::open(
        root,
        sender,
        token,
        std::env::var("VINYL_ALSA_DEVICE").ok(),
        std::env::var("VINYL_INPUT_DEVICE").ok(),
    )
    .await?;
    let bind = std::env::var("VINYL_BIND").unwrap_or_else(|_| "127.0.0.1:8790".into());
    controller.serve(&bind).await
}
