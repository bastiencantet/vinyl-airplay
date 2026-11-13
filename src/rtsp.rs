use crate::crypto::Cipher;
use anyhow::{Context, Result, anyhow, ensure};
use std::{
    collections::{BTreeMap, VecDeque},
    net::SocketAddr,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    time::timeout,
};

const MAX_HEADER: usize = 64 * 1024;
const MAX_BODY: usize = 1024 * 1024;
pub const DEADLINE: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub struct Message {
    pub start: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl Message {
    pub fn status(&self) -> Result<u16> {
        self.start
            .split_whitespace()
            .nth(1)
            .context("missing status")?
            .parse()
            .context("invalid status")
    }
    pub fn ok(self) -> Result<Self> {
        ensure!(
            self.status()? == 200,
            "receiver replied {}; authentication/access settings or timing mode may be unsupported",
            self.start
        );
        Ok(self)
    }
}

pub struct Channel {
    io: BufReader<TcpStream>,
    tx: Option<Cipher>,
    rx: Option<Cipher>,
    decoded: VecDeque<u8>,
}

impl Channel {
    pub async fn connect(addr: SocketAddr) -> Result<Self> {
        let io = timeout(DEADLINE, TcpStream::connect(addr))
            .await
            .context("TCP connect timeout")??;
        io.set_nodelay(true)?;
        Ok(Self {
            io: BufReader::new(io),
            tx: None,
            rx: None,
            decoded: VecDeque::new(),
        })
    }
    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.io.get_ref().local_addr()?)
    }
    pub fn enable(&mut self, tx: &[u8; 32], rx: &[u8; 32]) {
        self.tx = Some(Cipher::new(tx));
        self.rx = Some(Cipher::new(rx));
    }
    pub async fn write(&mut self, bytes: &[u8]) -> Result<()> {
        if let Some(cipher) = &mut self.tx {
            for frame in bytes.chunks(1024) {
                let length = (frame.len() as u16).to_le_bytes();
                let (sealed, _) = cipher.seal(frame, &length)?;
                self.io.get_mut().write_all(&length).await?;
                self.io.get_mut().write_all(&sealed).await?;
            }
        } else {
            self.io.get_mut().write_all(bytes).await?;
        }
        Ok(())
    }
    async fn byte(&mut self) -> Result<u8> {
        if self.rx.is_none() {
            return Ok(self.io.read_u8().await?);
        }
        if self.decoded.is_empty() {
            let mut length = [0; 2];
            self.io.read_exact(&mut length).await?;
            let len = u16::from_le_bytes(length) as usize;
            ensure!(
                (1..=1024).contains(&len),
                "invalid encrypted HAP record length {len}"
            );
            let mut record = vec![0; len + 16];
            self.io.read_exact(&mut record).await?;
            self.decoded
                .extend(self.rx.as_mut().unwrap().open(&record, &length)?);
        }
        self.decoded.pop_front().context("empty encrypted record")
    }
    pub async fn read(&mut self) -> Result<Message> {
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            ensure!(header.len() < MAX_HEADER, "RTSP header too large");
            header.push(self.byte().await?);
        }
        let text = std::str::from_utf8(&header).context("invalid RTSP header text")?;
        let mut lines = text[..text.len() - 4].split("\r\n");
        let start = lines.next().context("missing RTSP start line")?.to_string();
        let mut headers = BTreeMap::new();
        for line in lines {
            let (key, value) = line.split_once(':').context("invalid RTSP header")?;
            ensure!(
                headers
                    .insert(key.trim().to_ascii_lowercase(), value.trim().to_string())
                    .is_none(),
                "duplicate RTSP header"
            );
        }
        ensure!(
            !headers.contains_key("transfer-encoding"),
            "chunked RTSP bodies unsupported"
        );
        let len: usize = headers
            .get("content-length")
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(0);
        ensure!(len <= MAX_BODY, "RTSP body too large");
        let mut body = Vec::with_capacity(len);
        for _ in 0..len {
            body.push(self.byte().await?);
        }
        Ok(Message {
            start,
            headers,
            body,
        })
    }
}

pub struct Client {
    pub channel: Channel,
    cseq: u32,
    pub uri: String,
    pub id: u32,
    instance: String,
    session: Option<String>,
}

impl Client {
    pub async fn connect(addr: SocketAddr) -> Result<Self> {
        let channel = Channel::connect(addr).await?;
        let id = rand::random();
        let uri = format!("rtsp://{}/{id}", addr.ip());
        Ok(Self {
            channel,
            cseq: 0,
            uri,
            id,
            instance: format!("{:016X}", rand::random::<u64>()),
            session: None,
        })
    }
    pub fn sender_id(&self) -> &str {
        &self.instance
    }

    pub async fn request(
        &mut self,
        method: &str,
        path: Option<&str>,
        content_type: Option<&str>,
        body: &[u8],
        extra: &[(&str, String)],
    ) -> Result<Message> {
        self.exchange(method, path, content_type, body, extra, false)
            .await
    }

    pub async fn http(
        &mut self,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        body: &[u8],
        extra: &[(&str, String)],
    ) -> Result<Message> {
        self.exchange(method, Some(path), content_type, body, extra, true)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn exchange(
        &mut self,
        method: &str,
        path: Option<&str>,
        content_type: Option<&str>,
        body: &[u8],
        extra: &[(&str, String)],
        http: bool,
    ) -> Result<Message> {
        let cseq = self.cseq;
        self.cseq = self.cseq.checked_add(1).context("CSeq exhausted")?;
        let protocol = if http || (method == "POST" && path.is_some_and(|p| p.starts_with("/pair")))
        {
            "HTTP/1.1"
        } else {
            "RTSP/1.0"
        };
        let mut request = format!(
            "{method} {} {protocol}\r\nCSeq: {cseq}\r\nUser-Agent: AirPlay/550.10\r\nClient-Instance: {}\r\nDACP-ID: {}\r\nActive-Remote: {}\r\nConnection: keep-alive\r\nContent-Length: {}\r\n",
            path.unwrap_or(&self.uri),
            self.instance,
            self.instance,
            self.id,
            body.len()
        );
        if let Some(session) = &self.session {
            request.push_str(&format!("Session: {session}\r\n"));
        }
        if let Some(ct) = content_type {
            request.push_str(&format!("Content-Type: {ct}\r\n"));
        }
        for (key, value) in extra {
            ensure!(!value.contains(['\r', '\n']), "invalid header value");
            request.push_str(&format!("{key}: {value}\r\n"));
        }
        request.push_str("\r\n");
        let bytes = [request.as_bytes(), body].concat();
        let response = timeout(DEADLINE, async {
            self.channel.write(&bytes).await?;
            self.channel.read().await
        })
        .await
        .map_err(|_| anyhow!("{method} {} timed out", path.unwrap_or(&self.uri)))??;
        if let Some(received) = response.headers.get("cseq") {
            ensure!(received.parse::<u32>()? == cseq, "unexpected CSeq");
        }
        if let Some(session) = response.headers.get("session") {
            self.session = Some(session.split(';').next().unwrap().to_string());
        }
        Ok(response)
    }
    pub async fn plist(&mut self, method: &str, body: &plist::Value) -> Result<plist::Value> {
        let mut bytes = Vec::new();
        body.to_writer_binary(&mut bytes)?;
        let response = self
            .request(
                method,
                None,
                Some("application/x-apple-binary-plist"),
                &bytes,
                &[],
            )
            .await?
            .ok()?;
        plist::Value::from_reader(std::io::Cursor::new(response.body))
            .context("invalid receiver binary plist")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    #[tokio::test]
    async fn http_video_and_rtsp_control_keep_distinct_protocols() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (io, _) = listener.accept().await.unwrap();
            let mut channel = Channel {
                io: BufReader::new(io),
                tx: None,
                rx: None,
                decoded: VecDeque::new(),
            };
            for (cseq, expected) in ["GET /playback-info HTTP/1.1", "POST /feedback RTSP/1.0"]
                .into_iter()
                .enumerate()
            {
                let request = channel.read().await.unwrap();
                assert_eq!(request.start, expected);
                channel
                    .write(
                        format!("HTTP/1.1 200 OK\r\nCSeq: {cseq}\r\nContent-Length: 0\r\n\r\n")
                            .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        let mut client = Client::connect(address).await.unwrap();
        client
            .http("GET", "/playback-info", None, &[], &[])
            .await
            .unwrap()
            .ok()
            .unwrap();
        client
            .request("POST", Some("/feedback"), None, &[], &[])
            .await
            .unwrap()
            .ok()
            .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn encrypted_fragmentation_and_coalesced_responses() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (io, _) = listener.accept().await.unwrap();
            let mut channel = Channel {
                io: BufReader::new(io),
                tx: None,
                rx: None,
                decoded: VecDeque::new(),
            };
            channel.enable(&[9; 32], &[8; 32]);
            let message = channel.read().await.unwrap();
            assert_eq!(message.body, vec![3; 2500]);
            let both = b"RTSP/1.0 200 OK\r\nCSeq: 0\r\nContent-Length: 3\r\n\r\noneRTSP/1.0 200 OK\r\nContent-Length: 3\r\n\r\ntwo";
            channel.write(both).await.unwrap();
        });
        let mut channel = Channel::connect(address).await.unwrap();
        channel.enable(&[8; 32], &[9; 32]);
        let mut request = b"POST /test HTTP/1.1\r\nContent-Length: 2500\r\n\r\n".to_vec();
        request.extend_from_slice(&vec![3; 2500]);
        channel.write(&request).await.unwrap();
        assert_eq!(channel.read().await.unwrap().body, b"one");
        assert_eq!(channel.read().await.unwrap().body, b"two");
        server.await.unwrap();
    }
}
