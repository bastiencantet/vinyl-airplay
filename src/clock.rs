use anyhow::{Result, ensure};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const RATE: u32 = 44_100;
pub const FRAMES: u32 = 352;
pub const PACKET_BYTES: usize = FRAMES as usize * 2 * 2;

/// One sender clock shared by ALL receivers. Wall time is sampled only once.
#[derive(Clone)]
pub struct Timeline {
    pub start: Instant,
    pub rtp: u32,
    ntp: u64,
    rate: u32,
}

pub fn ntp_time(unix: Duration) -> u64 {
    ((unix.as_secs().wrapping_add(2_208_988_800) & 0xffff_ffff) << 32)
        | ((u64::from(unix.subsec_nanos()) << 32) / 1_000_000_000)
}

impl Timeline {
    pub fn new(rtp: u32) -> Self {
        Self::with_rate(rtp, RATE)
    }
    pub fn with_rate(rtp: u32, rate: u32) -> Self {
        Self {
            rate,
            start: Instant::now(),
            rtp,
            ntp: ntp_time(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock before Unix epoch"),
            ),
        }
    }
    pub fn ntp_at(&self, elapsed: Duration) -> u64 {
        self.ntp
            .wrapping_add(((elapsed.as_nanos() << 32) / 1_000_000_000) as u64)
    }
    pub fn now(&self) -> u64 {
        self.ntp_at(self.start.elapsed())
    }
    pub fn sync(&self, latency: u32, initial: bool) -> [u8; 20] {
        let elapsed = self.start.elapsed();
        let frames = (elapsed.as_nanos() * u128::from(self.rate) / 1_000_000_000) as u32;
        let head = self.rtp.wrapping_add(frames);
        let mut packet = [0; 20];
        packet[..4].copy_from_slice(&[if initial { 0x90 } else { 0x80 }, 0xd4, 0, 7]);
        packet[4..8].copy_from_slice(&head.wrapping_sub(latency).to_be_bytes());
        packet[8..16].copy_from_slice(&self.ntp_at(elapsed).to_be_bytes());
        packet[16..20].copy_from_slice(&head.to_be_bytes());
        packet
    }
}

pub fn latency_samples(ms: u32) -> Result<u32> {
    latency_samples_at(ms, RATE)
}

pub fn latency_samples_at(ms: u32, rate: u32) -> Result<u32> {
    ensure!(
        (1..=5000).contains(&ms),
        "latency must be between 1 and 5000 ms"
    );
    ensure!(rate == RATE || rate == 48000, "unsupported sample rate");
    Ok((u64::from(ms) * u64::from(rate)).div_ceil(1000) as u32)
}

pub fn timing_reply(request: &[u8], received: u64, sent: u64) -> Option<[u8; 32]> {
    if request.len() != 32 || request[1] & 0x7f != 0x52 {
        return None;
    }
    let mut reply = [0; 32];
    reply[..4].copy_from_slice(&[0x80, 0xd3, 0, 7]);
    reply[8..16].copy_from_slice(&request[24..32]);
    reply[16..24].copy_from_slice(&received.to_be_bytes());
    reply[24..32].copy_from_slice(&sent.to_be_bytes());
    Some(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ntp_epoch_fraction_and_samples() {
        assert_eq!(ntp_time(Duration::ZERO), 0x83aa7e80_00000000);
        assert_eq!(ntp_time(Duration::from_millis(1500)), 0x83aa7e81_80000000);
        assert_eq!(latency_samples(250).unwrap(), 11025);
        assert_eq!(latency_samples(1).unwrap(), 45);
        assert_eq!(latency_samples_at(77, 48000).unwrap(), 3696);
    }
    #[test]
    fn timing_request_origin_is_echoed() {
        let mut request = [0; 32];
        request[1] = 0xd2;
        request[24..32].copy_from_slice(&123u64.to_be_bytes());
        let reply = timing_reply(&request, 456, 789).unwrap();
        assert_eq!(u64::from_be_bytes(reply[8..16].try_into().unwrap()), 123);
        assert_eq!(u64::from_be_bytes(reply[16..24].try_into().unwrap()), 456);
        assert!(timing_reply(&request[..31], 0, 0).is_none());
    }
    #[test]
    fn sync_latency_survives_rtp_wrap() {
        let timeline = Timeline::new(5);
        let sync = timeline.sync(11025, true);
        let play = u32::from_be_bytes(sync[4..8].try_into().unwrap());
        let head = u32::from_be_bytes(sync[16..20].try_into().unwrap());
        assert_eq!(head.wrapping_sub(play), 11025);
        assert_eq!(sync[0], 0x90);
    }
}
