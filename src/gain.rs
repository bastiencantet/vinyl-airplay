//! Fixed digital gain, optionally chosen once from a short input calibration.

const AUTO_TARGET_RMS: f64 = 32768.0 * 0.1; // -20 dBFS
const AUTO_NOISE_THRESHOLD: f64 = 32768.0 * 0.0017782794100389228; // -55 dBFS
const AUTO_CEILING: f64 = 32768.0 * 0.35481338923357547; // -9 dBFS during calibration
const AUTO_MAX_GAIN: f64 = 31.622776601683793; // +30 dB
const CALIBRATION_SAMPLES: u64 = crate::clock::RATE as u64 * 2 * 5;

struct AutoGain {
    scale: f64,
    fixed: Option<f64>,
    samples: u64,
    power: f64,
    peak: u16,
    ramp_remaining: u32,
}

impl AutoGain {
    fn new() -> Self {
        Self {
            scale: 1.0,
            fixed: None,
            samples: 0,
            power: 0.0,
            peak: 0,
            ramp_remaining: 0,
        }
    }

    fn apply(&mut self, pcm: &mut [u8], peak: u16, rms: f64) -> u64 {
        if self.fixed.is_none() {
            // Ignore near-silence during measurement; never infer a huge gain from it.
            if rms >= AUTO_NOISE_THRESHOLD {
                let samples = (pcm.len() / 2) as u64;
                self.samples += samples;
                self.power += rms * rms * samples as f64;
                self.peak = self.peak.max(peak);
            }
            if self.samples < CALIBRATION_SAMPLES {
                return 0;
            }
            let measured_rms = (self.power / self.samples as f64).sqrt();
            let fixed = (AUTO_TARGET_RMS / measured_rms)
                .min(AUTO_CEILING / f64::from(self.peak))
                .min(AUTO_MAX_GAIN);
            self.fixed = Some(fixed);
            self.ramp_remaining = crate::clock::RATE / 4;
            eprintln!(
                "Input calibration complete: fixed gain {:+.1} dB; 9 dB headroom against observed peaks. Gain stays fixed; later louder passages can still clip.",
                20.0 * fixed.log10()
            );
        }
        let fixed = self.fixed.unwrap();
        let mut clipped = 0;
        for frame in pcm.as_chunks_mut::<4>().0 {
            // One short startup fade, then the gain stays identical for every sample.
            if self.ramp_remaining > 0 {
                self.scale += (fixed - self.scale) / f64::from(self.ramp_remaining);
                self.ramp_remaining -= 1;
                if self.ramp_remaining == 0 {
                    self.scale = fixed;
                }
            }
            // Both channels share the same fixed gain: no compressor or limiter.
            for bytes in frame.as_chunks_mut::<2>().0 {
                let sample = i16::from_be_bytes([bytes[0], bytes[1]]);
                let scaled = (f64::from(sample) * self.scale).round();
                if scaled > f64::from(i16::MAX) || scaled < f64::from(i16::MIN) {
                    clipped += 1;
                }
                bytes.copy_from_slice(
                    &(scaled.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16).to_be_bytes(),
                );
            }
        }
        clipped
    }
}

pub struct InputGain {
    db: i16,
    scale: f64,
    samples: u64,
    clipped: u64,
    peak: u16,
    power: f64,
    since_report: u64,
    auto: Option<AutoGain>,
}

impl InputGain {
    pub fn new(db: i16) -> Self {
        Self {
            db,
            scale: 10.0_f64.powf(f64::from(db) / 20.0),
            samples: 0,
            clipped: 0,
            peak: 0,
            power: 0.0,
            since_report: 0,
            auto: None,
        }
    }

    pub fn automatic() -> Self {
        let mut gain = Self::new(0);
        gain.auto = Some(AutoGain::new());
        gain
    }

    /// PCM is signed 16-bit big-endian, interleaved stereo.
    pub fn apply(&mut self, pcm: &mut [u8]) {
        let mut packet_peak = 0u16;
        let mut packet_power = 0.0;
        for bytes in pcm.as_chunks_mut::<2>().0 {
            let sample = i16::from_be_bytes([bytes[0], bytes[1]]);
            self.peak = self.peak.max(sample.unsigned_abs());
            packet_peak = packet_peak.max(sample.unsigned_abs());
            packet_power += f64::from(sample).powi(2);
            self.power += f64::from(sample).powi(2);
            self.samples += 1;
            self.since_report += 1;
            // Preserve every input bit at unity gain, including -32768.
            if self.auto.is_none() && self.db != 0 {
                let scaled = (f64::from(sample) * self.scale).round();
                if scaled > f64::from(i16::MAX) || scaled < f64::from(i16::MIN) {
                    self.clipped += 1;
                }
                let output = scaled.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16;
                bytes.copy_from_slice(&output.to_be_bytes());
            }
        }
        if let Some(auto) = &mut self.auto {
            let rms = (packet_power / (pcm.len() / 2).max(1) as f64).sqrt();
            self.clipped += auto.apply(pcm, packet_peak, rms);
        }
        if self.since_report >= u64::from(crate::clock::RATE) * 2 {
            self.report();
            self.since_report = 0;
        }
    }

    fn report(&self) {
        let peak_db = 20.0 * (f64::from(self.peak) / 32768.0).log10();
        let rms = (self.power / self.samples as f64).sqrt();
        let rms_db = 20.0 * (rms / 32768.0).log10();
        let gain_label = match &self.auto {
            Some(auto) if auto.fixed.is_none() => format!(
                "calibrating {:.1}/5.0 s of signal; 0 dB",
                auto.samples as f64 / (2.0 * f64::from(crate::clock::RATE))
            ),
            Some(auto) => format!("calibrated fixed {:+.1} dB", 20.0 * auto.scale.log10()),
            None => format!("{:+} dB", self.db),
        };
        eprintln!(
            "Input meter (cumulative, BEFORE gain): peak {peak_db:.1} dBFS, RMS {rms_db:.1} dBFS; gain {gain_label}; clipped samples: {}/{}{}",
            self.clipped,
            self.samples,
            if self.clipped > 0 {
                " — SATURATION: reduce --gain-db or the analog input level"
            } else {
                ""
            }
        );
    }
}

impl Drop for InputGain {
    fn drop(&mut self) {
        if self.samples > 0 {
            self.report();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(db: i16, samples: &[i16]) -> (Vec<i16>, u64) {
        let mut bytes: Vec<u8> = samples.iter().flat_map(|v| v.to_be_bytes()).collect();
        let mut gain = InputGain::new(db);
        gain.apply(&mut bytes);
        let result = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_be_bytes([b[0], b[1]]))
            .collect();
        (result, gain.clipped)
    }

    #[test]
    fn unity_preserves_all_pcm_values() {
        let samples: Vec<i16> = (i16::MIN..=i16::MAX).collect();
        let (output, clipped) = process(0, &samples);
        assert_eq!(output, samples);
        assert_eq!(clipped, 0);
    }

    #[test]
    fn gain_scales_both_channels_and_saturates_without_wrapping() {
        let (output, clipped) = process(20, &[0, 100, -100, 1000, -1000, 4000, -4000]);
        assert_eq!(output, [0, 1000, -1000, 10000, -10000, i16::MAX, i16::MIN]);
        assert_eq!(clipped, 2);
        let (output, clipped) = process(60, &[0, 0, 10, -10, 100, -100]);
        assert_eq!(output, [0, 0, 10000, -10000, i16::MAX, i16::MIN]);
        assert_eq!(clipped, 2);
    }

    #[test]
    fn attenuation_and_silent_input() {
        assert_eq!(process(-20, &[10000, -10000, 0]).0, [1000, -1000, 0]);
        assert_eq!(process(60, &[0; 8]), (vec![0; 8], 0));
    }

    #[test]
    fn automatic_gain_does_not_raise_silence_or_subthreshold_noise() {
        let mut gain = InputGain::automatic();
        let mut silence = [0; 1408];
        gain.apply(&mut silence);
        assert_eq!(silence, [0; 1408]);
        let noise: Vec<u8> = [10i16, -10]
            .iter()
            .cycle()
            .take(704)
            .flat_map(|v| v.to_be_bytes())
            .collect();
        for _ in 0..1000 {
            let mut bytes = noise.clone();
            gain.apply(&mut bytes);
            assert_eq!(bytes, noise);
        }
        assert_eq!(gain.auto.as_ref().unwrap().scale, 1.0);
    }

    fn stereo_packet(left: i16, right: i16) -> Vec<u8> {
        [left, right]
            .iter()
            .cycle()
            .take(704)
            .flat_map(|v| v.to_be_bytes())
            .collect()
    }

    #[test]
    fn calibration_chooses_gain_once_and_preserves_later_dynamics_and_stereo() {
        let mut gain = InputGain::automatic();
        let input = stereo_packet(1000, 500);
        for _ in 0..627 {
            gain.apply(&mut input.clone());
        }
        assert!(gain.auto.as_ref().unwrap().fixed.is_some());
        for _ in 0..40 {
            gain.apply(&mut input.clone());
        }
        let fixed = gain.auto.as_ref().unwrap().scale;
        for (left, right) in [(500i16, 250i16), (2000, 1000), (1000, 500)] {
            let mut packet = stereo_packet(left, right);
            gain.apply(&mut packet);
            assert_eq!(gain.auto.as_ref().unwrap().scale, fixed);
            assert_eq!(
                i16::from_be_bytes([packet[0], packet[1]]),
                (f64::from(left) * fixed).round() as i16
            );
            assert_eq!(
                i16::from_be_bytes([packet[2], packet[3]]),
                (f64::from(right) * fixed).round() as i16
            );
        }
        assert_eq!(gain.clipped, 0);
    }

    #[test]
    fn calibration_leaves_headroom_but_does_not_compress_later_transients() {
        let mut gain = InputGain::automatic();
        let input = stereo_packet(1000, -1000);
        for _ in 0..670 {
            gain.apply(&mut input.clone());
        }
        let fixed = gain.auto.as_ref().unwrap().scale;
        assert!((fixed * 1000.0 - AUTO_TARGET_RMS).abs() < 1.0);
        assert!(fixed * 1000.0 <= AUTO_CEILING);
        let mut transient = stereo_packet(i16::MAX, i16::MIN);
        gain.apply(&mut transient);
        assert_eq!(gain.auto.as_ref().unwrap().scale, fixed);
        assert!(gain.clipped > 0); // reported; deliberately not an automatic limiter
        let mut silence = stereo_packet(0, 0);
        gain.apply(&mut silence);
        assert_eq!(gain.auto.as_ref().unwrap().scale, fixed);
        assert!(silence.iter().all(|b| *b == 0));
        let mut low = AutoGain::new();
        for _ in 0..630 {
            low.apply(&mut stereo_packet(60, -60), 60, 60.0);
        }
        assert_eq!(low.fixed, Some(AUTO_MAX_GAIN));
    }
}
