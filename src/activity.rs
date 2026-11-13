//! Input activity detection before digital gain; no HomePod session while idle.
use crate::clock::RATE;

#[derive(Clone, Copy)]
pub struct Settings {
    pub threshold_dbfs: i16,
    pub silence_seconds: u32,
}

impl Settings {
    pub fn off_dbfs(self) -> f64 {
        f64::from(self.threshold_dbfs) - 5.0
    }
}

pub fn rms_dbfs(pcm: &[u8]) -> f64 {
    let mut power = 0.0;
    let mut count = 0;
    for sample in pcm.chunks_exact(2) {
        let value = f64::from(i16::from_be_bytes([sample[0], sample[1]])) / 32768.0;
        power += value * value;
        count += 1;
    }
    if count == 0 || power == 0.0 {
        f64::NEG_INFINITY
    } else {
        10.0 * (power / f64::from(count)).log10()
    }
}

pub struct Detector {
    settings: Settings,
    active_frames: u64,
}

impl Detector {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            active_frames: 0,
        }
    }

    pub fn observe(&mut self, pcm: &[u8]) -> bool {
        if rms_dbfs(pcm) >= f64::from(self.settings.threshold_dbfs) {
            self.active_frames += (pcm.len() / 4) as u64;
        } else {
            self.active_frames = 0;
        }
        self.active_frames >= u64::from(RATE) * 300 / 1000
    }
}

pub struct SilenceDetector {
    settings: Settings,
    quiet_frames: u64,
}

impl SilenceDetector {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            quiet_frames: 0,
        }
    }

    pub fn observe(&mut self, pcm: &[u8]) -> bool {
        if rms_dbfs(pcm) >= self.settings.off_dbfs() {
            self.quiet_frames = 0;
        } else {
            self.quiet_frames += (pcm.len() / 4) as u64;
        }
        self.quiet_frames >= u64::from(self.settings.silence_seconds) * u64::from(RATE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(sample: i16, frames: usize) -> Vec<u8> {
        sample.to_be_bytes().repeat(frames * 2)
    }
    #[test]
    fn isolated_clicks_and_background_noise_do_not_trigger() {
        let mut detector = Detector::new(Settings {
            threshold_dbfs: -50,
            silence_seconds: 60,
        });
        for _ in 0..100 {
            assert!(!detector.observe(&packet(1, 352)));
        }
        assert!(!detector.observe(&packet(10000, 352)));
        assert!(!detector.observe(&packet(1, 352)));
        assert!(!detector.observe(&packet(10000, 352)));
    }
    #[test]
    fn trigger_uses_audio_duration_and_ignores_digital_gain() {
        let mut detector = Detector::new(Settings {
            threshold_dbfs: -50,
            silence_seconds: 60,
        });
        assert!(!detector.observe(&packet(1000, 13229)));
        assert!(detector.observe(&packet(1000, 1)));
        assert!(!detector.observe(&packet(0, 352)));
    }
    #[test]
    fn release_waits_sixty_seconds_and_music_resets_it() {
        let mut detector = SilenceDetector::new(Settings {
            threshold_dbfs: -50,
            silence_seconds: 60,
        });
        let quiet = packet(0, RATE as usize);
        for _ in 0..59 {
            assert!(!detector.observe(&quiet));
        }
        // A passage below the start threshold but above the stop threshold keeps playback alive.
        assert!(!detector.observe(&packet(80, 352)));
        for _ in 0..59 {
            assert!(!detector.observe(&quiet));
        }
        assert!(detector.observe(&quiet));
    }
    #[test]
    fn rms_scale_and_release_hysteresis() {
        assert_eq!(rms_dbfs(&packet(0, 352)), f64::NEG_INFINITY);
        assert!((rms_dbfs(&packet(3276, 352)) + 20.0).abs() < 0.01);
        assert_eq!(
            Settings {
                threshold_dbfs: -50,
                silence_seconds: 60
            }
            .off_dbfs(),
            -55.0
        );
    }
}
