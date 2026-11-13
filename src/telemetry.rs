//! Stereo spectrum and meters, five times per second. The PCM is never changed.
const N: usize = 2048;
const EDGES: [f64; 13] = [
    40., 80., 160., 250., 400., 630., 1000., 1600., 2500., 4000., 6300., 10000., 18000.,
];
pub struct Meter {
    count: usize,
    power: [f64; 2],
    peak: [u16; 2],
    samples: Vec<[f64; 2]>,
}
impl Default for Meter {
    fn default() -> Self {
        Self {
            count: 0,
            power: [0.; 2],
            peak: [0; 2],
            samples: Vec::with_capacity(N),
        }
    }
}
fn db(value: f64) -> f64 {
    20. * value.max(1e-6).log10()
}
impl Meter {
    pub fn observe(&mut self, pcm: &[u8]) {
        for frame in pcm.as_chunks::<4>().0 {
            let mut pair = [0.; 2];
            for c in 0..2 {
                let s = i16::from_be_bytes([frame[c * 2], frame[c * 2 + 1]]);
                pair[c] = f64::from(s) / 32768.;
                self.peak[c] = self.peak[c].max(s.unsigned_abs());
                self.power[c] += pair[c] * pair[c];
            }
            self.count += 1;
            if self.samples.len() < N {
                self.samples.push(pair);
            }
        }
        if self.count >= 8820 {
            let rms = self.power.map(|p| db((p / self.count as f64).sqrt()));
            let peak = self.peak.map(|p| db(f64::from(p) / 32768.));
            eprintln!(
                "control_event={}",
                serde_json::json!({"type":"meter", "rms_dbfs":db(((self.power[0]+self.power[1])/(2*self.count) as f64).sqrt()), "peak_dbfs":peak[0].max(peak[1]), "channels_rms_dbfs":rms,"channels_peak_dbfs":peak,"spectrum_dbfs":spectrum(&self.samples)})
            );
            self.count = 0;
            self.power = [0.; 2];
            self.peak = [0; 2];
            self.samples.clear();
        }
    }
}
fn spectrum(samples: &[[f64; 2]]) -> [f64; 12] {
    let mut power = [0.; N / 2];
    for channel in 0..2 {
        let mut re = [0.; N];
        let mut im = [0.; N];
        for (i, frame) in samples.iter().take(N).enumerate() {
            re[i] = frame[channel]
                * (0.5 - 0.5 * (2. * std::f64::consts::PI * i as f64 / N as f64).cos());
        }
        for i in 0..N {
            let j = i.reverse_bits() >> (usize::BITS - N.trailing_zeros());
            if i < j {
                re.swap(i, j);
            }
        }
        let mut size = 2;
        while size <= N {
            let angle = -2. * std::f64::consts::PI / size as f64;
            let (sin, cos) = angle.sin_cos();
            for start in (0..N).step_by(size) {
                let (mut wr, mut wi) = (1., 0.);
                for offset in 0..size / 2 {
                    let a = start + offset;
                    let b = a + size / 2;
                    let (tr, ti) = (wr * re[b] - wi * im[b], wr * im[b] + wi * re[b]);
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                    (wr, wi) = (wr * cos - wi * sin, wr * sin + wi * cos);
                }
            }
            size *= 2;
        }
        for bin in 1..N / 2 {
            power[bin] += (re[bin] * re[bin] + im[bin] * im[bin]) / (N * N) as f64 / 0.375;
        }
    }
    std::array::from_fn(|band| {
        let sum: f64 = power
            .iter()
            .enumerate()
            .filter(|(bin, _)| {
                let hz = *bin as f64 * 44100. / N as f64;
                hz >= EDGES[band] && hz < EDGES[band + 1]
            })
            .map(|(_, p)| p)
            .sum();
        db(sum.sqrt())
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_frequencies_survive_stereo_phase_inversion() {
        let samples: Vec<_> = (0..N)
            .map(|i| {
                let s = 0.5 * (2. * std::f64::consts::PI * 1200. * i as f64 / 44100.).sin();
                [s, -s]
            })
            .collect();
        let bands = spectrum(&samples);
        assert_eq!(
            bands
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap()
                .0,
            6
        );
        assert!((bands[6] + 9.03).abs() < 0.2, "{bands:?}");
        assert!(spectrum(&vec![[0.; 2]; N]).iter().all(|v| *v == -120.));
    }
}
