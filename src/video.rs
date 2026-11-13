//! Video-only HTTP media bench, isolated from the live vinyl/audio process.
use crate::{
    clock::{Timeline, timing_reply},
    crypto::derive,
    pairing,
    rtsp::{Channel, Client},
};
use anyhow::{Context, Result, ensure};
use plist::{Dictionary, Value};
use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};
use tokio::net::UdpSocket;

fn dict<const N: usize>(entries: [(&str, Value); N]) -> Value {
    Value::Dictionary(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect::<Dictionary>(),
    )
}

pub async fn bench(target: SocketAddr, url: String, seconds: u64) -> Result<()> {
    ensure!(
        url.starts_with("http://") || url.starts_with("https://"),
        "video URL must use HTTP(S)"
    );
    let mut client = Client::connect(target).await?;
    let secret = pairing::transient(&mut client).await?;
    let local = client.channel.local_addr()?.ip();
    let timing = UdpSocket::bind(SocketAddr::new(local, 0)).await?;
    let timing_port = timing.local_addr()?.port();
    let peer = target.ip();
    let timing_task = tokio::spawn(async move {
        let clock = Timeline::new(0);
        let mut bytes = [0u8; 2048];
        loop {
            let (size, from) = timing.recv_from(&mut bytes).await?;
            if from.ip() == peer
                && let Some(reply) = timing_reply(&bytes[..size], clock.now(), clock.now())
            {
                timing.send_to(&reply, from).await?;
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    });
    let mut event_task = None;
    let mut played = false;
    let started = Instant::now();
    let session = uuid::Uuid::new_v4().to_string();
    let headers = [
        ("X-Apple-Session-ID", session.clone()),
        ("X-Apple-ProtocolVersion", "1".into()),
        ("X-Apple-Stream-ID", "1".into()),
    ];
    let result = async {
        let setup = client.plist("SETUP", &dict([
            ("deviceID", "02:56:49:44:45:4F".into()),
            ("macAddress", "02:56:49:44:45:4F".into()),
            ("sessionUUID", session.clone().into()),
            ("name", "Vinyl silent video benchmark".into()),
            ("model", "iPhone14,3".into()),
            ("timingProtocol", "NTP".into()),
            ("timingPort", u64::from(timing_port).into()),
            ("isMultiSelectAirPlay", true.into()),
            ("groupContainsGroupLeader", false.into()),
            ("senderSupportsRelay", false.into()),
        ])).await?;
        let port = setup.as_dictionary().and_then(|d| d.get("eventPort")).and_then(Value::as_unsigned_integer).context("video eventPort missing")?;
        let mut event = Channel::connect(SocketAddr::new(peer, u16::try_from(port)?)).await?;
        event.enable(&derive(&secret,"Events-Salt","Events-Read-Encryption-Key"), &derive(&secret,"Events-Salt","Events-Write-Encryption-Key"));
        event_task = Some(tokio::spawn(async move {
            loop {
                let request = event.read().await?;
                let mut response = String::from("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n");
                if let Some(cseq) = request.headers.get("cseq") { response.push_str(&format!("CSeq: {}\r\n", cseq.parse::<u32>()?)); }
                response.push_str("\r\n");
                event.write(response.as_bytes()).await?;
                eprintln!("video_event={}", serde_json::json!({"start":request.start,"bytes":request.body.len()}));
            }
            #[allow(unreachable_code)] Ok::<(), anyhow::Error>(())
        }));
        client.request("RECORD",None,None,&[],&[]).await?.ok()?;
        let mut body = Vec::new();
        dict([
            ("Content-Location", url.into()),
            ("Start-Position-Seconds", Value::Real(0.0)),
            ("uuid", session.into()),
            ("streamType", 1u64.into()),
            ("mediaType", "file".into()),
            ("rate", Value::Real(1.0)),
            ("clientBundleID", "dev.pyatv.GPU".into()),
            ("clientProcName", "dev.pyatv.GPU".into()),
            ("model", "iPhone14,3".into()),
            ("osBuildVersion", "20G1116".into()),
            ("SenderMACAddress", "02:56:49:44:45:4F".into()),
            ("mightSupportStorePastisKeyRequests", true.into()),
            ("playbackRestrictions", 0u64.into()),
            ("referenceRestrictions", 3u64.into()),
        ]).to_writer_binary(&mut body)?;
        let play_start = Instant::now();
        let response = client.http("POST","/play",Some("application/x-apple-binary-plist"),&body,&headers).await?;
        println!("{}",serde_json::json!({"phase":"play_response","elapsed_ms":play_start.elapsed().as_secs_f64()*1000.0,"status":response.status()?,"setup_ms":play_start.duration_since(started).as_secs_f64()*1000.0}));
        response.ok()?;
        played = true;
        let rate = client.http("POST","/rate?value=1.000000",None,&[],&headers).await?;
        println!("{}",serde_json::json!({"phase":"rate_response","status":rate.status()?}));
        rate.ok()?;
        let interrupt = tokio::signal::ctrl_c();
        tokio::pin!(interrupt);
        let mut count = 0;
        while play_start.elapsed() < Duration::from_secs(seconds) {
            tokio::select! { _ = &mut interrupt => break, _ = tokio::time::sleep(Duration::from_millis(500)) => {} }
            let before = play_start.elapsed().as_secs_f64();
            let response = client.http("GET","/playback-info",None,&[],&headers).await?;
            let status = response.status()?;
            let info = if status == 200 && !response.body.is_empty() { plist::from_bytes::<Value>(&response.body).ok().and_then(|p| serde_json::to_value(p).ok()) } else { None };
            println!("{}", serde_json::json!({"phase":"playback_info","elapsed_s":play_start.elapsed().as_secs_f64(),"query_rtt_ms":(play_start.elapsed().as_secs_f64()-before)*1000.0,"status":status,"info":info}));
            count += 1;
            if count % 4 == 0 { let _ = client.request("POST",Some("/feedback"),None,&[],&[]).await?; }
        }
        Ok(())
    }.await;
    if played {
        let stopped = client.http("POST", "/stop", None, &[], &headers).await;
        eprintln!("video_stop={:?}", stopped.map(|r| r.status()));
    }
    let teardown = client.request("TEARDOWN", None, None, &[], &[]).await;
    eprintln!("video_teardown={:?}", teardown.map(|r| r.status()));
    if let Some(task) = event_task {
        task.abort();
        let _ = task.await;
    }
    timing_task.abort();
    let _ = timing_task.await;
    result
}
