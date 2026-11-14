//! Reusable host-audio and local-control infrastructure for the AirPlay sender.
pub mod audio;
pub mod ipc;

/// Persistent shutdown state: requests cannot be lost between stream phases.
pub mod shutdown {
    use std::sync::OnceLock;
    use tokio::sync::watch;
    fn channel() -> &'static watch::Sender<bool> {
        static CHANNEL: OnceLock<watch::Sender<bool>> = OnceLock::new();
        CHANNEL.get_or_init(|| watch::channel(false).0)
    }
    pub fn request() {
        channel().send_replace(true);
    }
    pub async fn requested() {
        let mut rx = channel().subscribe();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = rx.wait_for(|value| *value) => {},
        }
    }
}
pub mod activity;
pub mod clock;
mod crypto;
pub mod discovery;
mod gain;
mod metadata;
pub mod pairing;
pub mod recognition;
pub mod rtsp;
pub mod stream;
mod telemetry;
mod tlv;
pub mod video;
mod visualization;
pub mod volume;
