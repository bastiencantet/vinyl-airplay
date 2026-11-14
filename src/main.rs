use vinyl_airplay::{
    activity, clock, discovery, pairing, recognition, rtsp, stream, video, volume,
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::{net::SocketAddr, time::Duration};

#[derive(Parser)]
#[command(
    version,
    about = "Native Rust AirPlay research sender for vinyl → HomePod. Experimental: acoustic latency and long-term stability not measured."
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List native input devices and the compiled host backend as JSON.
    AudioInputs,
    /// Discover _airplay and _raop Bonjour services; print raw TXT capabilities as JSON.
    Discover {
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..=60))]
        seconds: u64,
    },
    /// Read receiver /info as JSON. Does not start playback or change volume.
    Probe {
        /// Address and advertised AirPlay port, e.g. 192.168.1.20:7000.
        target: SocketAddr,
        /// Establish a transient authenticated encrypted session before GET /info.
        #[arg(long)]
        authenticate: bool,
    },
    /// Video-only URL playback benchmark. Reports receiver state, not measured screen latency.
    VideoBench {
        target: SocketAddr,
        url: String,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u64).range(1..=120))]
        seconds: u64,
    },
    /// Recognize a captured PCM excerpt without starting AirPlay playback.
    Identify {
        /// 6..12 seconds of PCM S16BE/44100/stereo; only the fingerprint is uploaded.
        pcm_file: std::path::PathBuf,
    },
    /// EXPERIMENTAL: stream PCM S16BE/44100/stereo via AirPlay 2/NTP.
    #[command(group(clap::ArgGroup::new("live_input").args(["alsa_device", "input_device"])))]
    Stream {
        /// Repeat for both HomePods. Use the port advertised by _airplay._tcp.
        #[arg(long, required = true)]
        target: Vec<SocketAddr>,
        /// Native input ID from audio-inputs, or "default" (Core Audio / WASAPI / ALSA).
        #[arg(long, conflicts_with = "alsa_device")]
        input_device: Option<String>,
        /// Legacy Linux ALSA input (e.g. plughw:CARD=USB,DEV=0).
        /// Without this option, reads raw PCM from stdin.
        #[arg(long)]
        alsa_device: Option<String>,
        /// RESEARCH ONLY: stdin contains u16-BE lengths + raw AAC-ELD access units.
        /// 48000 Hz stereo, 480 frames/unit. No live encoder or decoder validation.
        #[arg(long, conflicts_with_all = ["alsa_device", "input_device", "gain_db", "auto_gain"])]
        preencoded_aac_eld: bool,
        /// RESEARCH ONLY: u16-BE lengths + raw compressed stereo access units.
        /// ALAC: 44100/352; AAC-LC: 44100/1024; AAC-ELD: 48000/480. No live encoder.
        #[arg(long, value_enum, conflicts_with_all = ["preencoded_aac_eld", "alsa_device", "input_device", "gain_db", "auto_gain"])]
        preencoded_codec: Option<stream::Codec>,
        /// Fixed digital input gain, -60..60 dB. Positive gain can clip and amplifies noise.
        #[arg(long, default_value_t = 0, allow_hyphen_values = true, conflicts_with = "auto_gain", value_parser = clap::value_parser!(i16).range(-60..=60))]
        gain_db: i16,
        /// Measure 5 seconds of signal, then hold a fixed stereo gain (max +30 dB).
        /// Leaves 9 dB headroom against measured peaks; later louder input can clip.
        #[arg(long)]
        auto_gain: bool,
        /// Monitor a live input without a receiver session; connect on activity and release on silence.
        #[arg(long, requires = "live_input")]
        auto_connect: bool,
        /// Identify vinyl songs via Shazam fingerprints and publish title, album and artwork.
        #[arg(long, requires = "live_input")]
        recognize: bool,
        /// Trigger threshold measured on raw input, BEFORE gain. Requires 300 ms of signal.
        #[arg(long, default_value_t = -50, allow_hyphen_values = true, requires = "auto_connect", value_parser = clap::value_parser!(i16).range(-90..=-10))]
        activity_threshold_dbfs: i16,
        /// Release receivers after this many seconds below trigger threshold minus 5 dB.
        #[arg(long, default_value_t = 60, requires = "auto_connect", value_parser = clap::value_parser!(u32).range(5..=3600))]
        silence_seconds: u32,
        /// Reserve two consecutive incoming UDP ports per receiver (timing, control).
        /// Useful for a narrow host firewall rule. Otherwise uses ephemeral ports.
        #[arg(long, value_parser = clap::value_parser!(u16).range(1024..=65520))]
        udp_base_port: Option<u16>,
        /// Requested schedule, NOT a measured delay or a guaranteed receiver minimum.
        #[arg(long, default_value_t = 250, value_parser = clap::value_parser!(u32).range(1..=5000))]
        latency_ms: u32,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Args::parse().command {
        Command::AudioInputs => println!(
            "{}",
            serde_json::to_string_pretty(&vinyl_airplay::audio::devices()?)?
        ),
        Command::VideoBench {
            target,
            url,
            seconds,
        } => {
            video::bench(target, url, seconds).await?;
        }
        Command::Identify { pcm_file } => {
            let size = std::fs::metadata(&pcm_file)?.len();
            anyhow::ensure!(
                (6 * 44100 * 4..=12 * 44100 * 4).contains(&size),
                "identify expects 6..12 seconds of PCM S16BE/44100/stereo"
            );
            let pcm = std::fs::read(pcm_file)?;
            let track = recognition::identify(&pcm).await?;
            println!("{}", serde_json::to_string_pretty(&track.map(|track| serde_json::json!({"key":track.key,"title":track.title,"artist":track.artist,"album":track.album,"artwork_bytes":track.artwork.as_ref().map_or(0, Vec::len)})))?);
        }
        Command::Discover { seconds } => {
            let devices = tokio::task::spawn_blocking(move || {
                discovery::discover(Duration::from_secs(seconds))
            })
            .await??;
            println!("{}", serde_json::to_string_pretty(&devices)?);
        }
        Command::Probe {
            target,
            authenticate,
        } => {
            let mut client = rtsp::Client::connect(target).await?;
            if authenticate {
                pairing::transient(&mut client).await?;
                eprintln!(
                    "Transient SRP pairing completed; server proof verified; control channel encrypted."
                );
            }
            let response = client.request("GET", Some("/info"), None, &[], &[]).await?;
            let status = response.status()?;
            let info = if status == 200 && !response.body.is_empty() {
                let value = plist::Value::from_reader(std::io::Cursor::new(&response.body))
                    .context("/info did not return a valid plist")?;
                serde_json::to_value(value)?
            } else {
                serde_json::Value::Null
            };
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"target": target.to_string(), "authenticated": authenticate, "status": status, "headers": response.headers, "info": info})
                )?
            );
            anyhow::ensure!(
                status == 200,
                "/info rejected with {status}; inspect the report above"
            );
        }
        Command::Stream {
            target,
            latency_ms,
            alsa_device,
            input_device,
            preencoded_aac_eld,
            preencoded_codec,
            gain_db,
            auto_gain,
            auto_connect,
            recognize,
            activity_threshold_dbfs,
            silence_seconds,
            udp_base_port,
        } => {
            eprintln!(
                "Experimental AirPlay 2/NTP sender. Acoustic latency and inter-speaker synchronization require measurement on your HomePods."
            );
            let codec = preencoded_codec.unwrap_or(if preencoded_aac_eld {
                stream::Codec::AacEld
            } else {
                stream::Codec::Pcm
            });
            anyhow::ensure!(
                preencoded_codec != Some(stream::Codec::Pcm),
                "preencoded codec must be compressed"
            );
            if recognize {
                anyhow::ensure!(
                    recognition::fingerprint_program()?.is_file(),
                    "install vinyl-shazam-fingerprint next to this executable before --recognize"
                );
            }
            let _volume_server = volume::start()?;
            let source = if codec != stream::Codec::Pcm {
                stream::InputSource::Encoded(codec)
            } else if let Some(device) = input_device {
                stream::InputSource::Native(device)
            } else if let Some(device) = alsa_device {
                stream::InputSource::Alsa(device)
            } else {
                stream::InputSource::Stdin
            };
            if auto_connect {
                return stream::monitor(
                    source,
                    target,
                    clock::latency_samples(latency_ms)?,
                    udp_base_port,
                    gain_db,
                    auto_gain,
                    recognize,
                    activity::Settings {
                        threshold_dbfs: activity_threshold_dbfs,
                        silence_seconds,
                    },
                )
                .await;
            }
            stream::play(
                source,
                target,
                if codec == stream::Codec::Pcm {
                    clock::latency_samples(latency_ms)?
                } else {
                    clock::latency_samples_at(latency_ms, codec.rate())?
                },
                udp_base_port,
                gain_db,
                auto_gain,
                recognize,
            )
            .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_sources_allow_activity_but_cannot_be_mixed() {
        for input in ["--input-device", "--alsa-device"] {
            assert!(
                Args::try_parse_from([
                    "vinyl-airplay",
                    "stream",
                    "--target",
                    "127.0.0.1:7000",
                    input,
                    "default",
                    "--auto-connect",
                    "--recognize"
                ])
                .is_ok()
            );
        }
        for tail in [
            vec!["--input-device", "default", "--alsa-device", "default"],
            vec!["--input-device", "default", "--preencoded-aac-eld"],
            vec!["--auto-connect"],
        ] {
            let mut args = vec!["vinyl-airplay", "stream", "--target", "127.0.0.1:7000"];
            args.extend(tail);
            assert!(Args::try_parse_from(args).is_err());
        }
    }
}
