//! Optional local PCM copy for the video sidecar. No waits or input modification.
//! Integrate only in the PCM path, before digital gain; see integration.patch.
use std::{
    net::{SocketAddr, UdpSocket},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Tap {
    socket: UdpSocket,
    packet: [u8; 1432],
    sequence: u64,
}
impl Tap {
    pub fn from_env() -> Option<Self> {
        let address = std::env::var("VINYL_VISUALIZER_UDP").ok()?;
        match Self::connect(&address) {
            Ok(tap) => Some(tap),
            Err(error) => {
                eprintln!("Video visualizer disabled: {error}");
                None
            }
        }
    }
    fn connect(address: &str) -> std::io::Result<Self> {
        let peer: SocketAddr = address
            .parse()
            .map_err(|_| std::io::Error::other("expected loopback IP:port"))?;
        if !peer.ip().is_loopback() {
            return Err(std::io::Error::other("visualizer accepts loopback only"));
        }
        let local = if peer.is_ipv4() {
            "127.0.0.1:0"
        } else {
            "[::1]:0"
        };
        let socket = UdpSocket::bind(local)?;
        socket.connect(peer)?;
        socket.set_nonblocking(true)?;
        let mut packet = [0; 1432];
        packet[..4].copy_from_slice(b"VPCM");
        packet[20..24].copy_from_slice(&44100u32.to_be_bytes());
        Ok(Self {
            socket,
            packet,
            sequence: 0,
        })
    }
    pub fn observe(&mut self, pcm: &[u8]) {
        if pcm.is_empty() || pcm.len() > 1408 || !pcm.len().is_multiple_of(4) {
            return;
        }
        self.packet[4..12].copy_from_slice(&self.sequence.to_be_bytes());
        self.sequence = self.sequence.wrapping_add(1);
        let micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;
        self.packet[12..20].copy_from_slice(&micros.to_be_bytes());
        self.packet[24..24 + pcm.len()].copy_from_slice(pcm);
        // Best effort: a slow, absent or failed renderer cannot wait on the audio loop.
        let _ = self.socket.send(&self.packet[..24 + pcm.len()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_copy_preserves_pcm_and_sequence() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        let mut tap = Tap::connect(&receiver.local_addr().unwrap().to_string()).unwrap();
        let pcm = [0, 1, 0xff, 0xfe];
        let mut packet = [0; 1500];
        for sequence in 0u64..2 {
            tap.observe(&pcm);
            let size = receiver.recv(&mut packet).unwrap();
            assert_eq!(size, 28);
            assert_eq!(&packet[..4], b"VPCM");
            assert_eq!(&packet[4..12], &sequence.to_be_bytes());
            assert_eq!(&packet[20..24], &44100u32.to_be_bytes());
            assert_eq!(&packet[24..size], &pcm);
        }
        assert_eq!(pcm, [0, 1, 0xff, 0xfe]);
    }
    #[test]
    fn refuses_nonlocal_destinations() {
        assert!(Tap::connect("192.168.50.30:8792").is_err());
        assert!(Tap::connect("not-an-address").is_err());
    }
    #[test]
    fn missing_consumer_never_fails_audio() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        let mut tap = Tap::connect(&receiver.local_addr().unwrap().to_string()).unwrap();
        drop(receiver);
        for _ in 0..1000 {
            tap.observe(&[0; 1408]);
        }
    }
}
