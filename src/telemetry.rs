//! Shared stereo spectrum at 30 Hz, analyzed off the audio loop. The PCM is never changed.
const N: usize = 4096;
const EDGES: [f64; 21] = [
    32.000000000000,
    44.151349166759,
    60.916926038908,
    84.048889740921,
    115.964746188431,
    160.000000000000,
    220.756745833794,
    304.584630194541,
    420.244448704603,
    579.823730942156,
    800.000000000000,
    1103.783729168972,
    1522.923150972703,
    2101.222243523014,
    2899.118654710781,
    4000.000000000000,
    5518.918645844861,
    7614.615754863513,
    10506.111217615071,
    14495.593273553906,
    20000.000000000000,
];
const HOP: usize = 44100 / 30;
struct Snapshot {
    samples: Vec<[f64; 2]>,
    power: [f64; 2],
    peak: [u16; 2],
}
pub struct Meter {
    count: usize,
    power: [f64; 2],
    peak: [u16; 2],
    samples: Vec<[f64; 2]>,
    next: usize,
    output: std::sync::mpsc::SyncSender<Snapshot>,
}
impl Default for Meter {
    fn default() -> Self {
        let (meter, receiver) = Self::channel();
        std::thread::spawn(move || {
            while let Ok(snapshot) = receiver.recv() {
                let rms = snapshot.power.map(|p| db((p / HOP as f64).sqrt()));
                let peak = snapshot.peak.map(|p| db(f64::from(p) / 32768.));
                eprintln!(
                    "control_event={}",
                    serde_json::json!({
                        "type":"meter", "rms_dbfs":db(((snapshot.power[0]+snapshot.power[1])/(2*HOP) as f64).sqrt()),
                        "peak_dbfs":peak[0].max(peak[1]), "channels_rms_dbfs":rms,
                        "channels_peak_dbfs":peak, "spectrum_dbfs":spectrum(&snapshot.samples)
                    })
                );
            }
        });
        meter
    }
}
fn db(value: f64) -> f64 {
    20. * value.max(1e-6).log10()
}
impl Meter {
    fn channel() -> (Self, std::sync::mpsc::Receiver<Snapshot>) {
        let (output, receiver) = std::sync::mpsc::sync_channel(1);
        (
            Self {
                count: 0,
                power: [0.; 2],
                peak: [0; 2],
                samples: vec![[0.; 2]; N],
                next: 0,
                output,
            },
            receiver,
        )
    }
    pub fn observe(&mut self, pcm: &[u8]) {
        for frame in pcm.as_chunks::<4>().0 {
            let mut pair = [0.; 2];
            for c in 0..2 {
                let sample = i16::from_be_bytes([frame[c * 2], frame[c * 2 + 1]]);
                pair[c] = f64::from(sample) / 32768.;
                self.peak[c] = self.peak[c].max(sample.unsigned_abs());
                self.power[c] += pair[c] * pair[c];
            }
            self.samples[self.next] = pair;
            self.next = (self.next + 1) % N;
            self.count += 1;
            if self.count == HOP {
                // Overlapping, latest 4096-frame window, independent of packet boundaries.
                let samples = self.samples[self.next..]
                    .iter()
                    .chain(&self.samples[..self.next])
                    .copied()
                    .collect();
                // A delayed analyzer drops this display update, never waits on playback.
                let _ = self.output.try_send(Snapshot {
                    samples,
                    power: self.power,
                    peak: self.peak,
                });
                self.count = 0;
                self.power = [0.; 2];
                self.peak = [0; 2];
            }
        }
    }
}
fn spectrum(samples: &[[f64; 2]]) -> [f64; 20] {
    let mut power = [0.; N / 2];
    let normalization = 1. / (N as f64 * (3. * (N - 1) as f64 / 8.));
    for channel in 0..2 {
        let mut re = [0.; N];
        let mut im = [0.; N];
        for (i, frame) in samples.iter().take(N).enumerate() {
            re[i] = frame[channel]
                * (0.5 - 0.5 * (2. * std::f64::consts::PI * i as f64 / (N - 1) as f64).cos());
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
            power[bin] += (re[bin] * re[bin] + im[bin] * im[bin]) * normalization;
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
    fn publishes_thirty_overlapping_windows_independent_of_packet_boundaries() {
        let (mut meter, receiver) = Meter::channel();
        let pcm: Vec<_> = (0..44100u32)
            .flat_map(|i| {
                let sample = (i as i16).to_be_bytes();
                [sample[0], sample[1], sample[0], sample[1]]
            })
            .collect();
        let mut frames = Vec::new();
        for chunk in pcm.chunks(352 * 4) {
            meter.observe(chunk);
            frames.extend(receiver.try_iter());
        }
        assert_eq!(frames.len(), 30);
        let last = &frames.last().unwrap().samples;
        assert_eq!(last.len(), 4096);
        for (i, pair) in last.iter().enumerate() {
            let expected = f64::from((44100 - 4096 + i) as i16) / 32768.;
            assert_eq!(*pair, [expected, expected]);
        }
    }
    #[test]
    fn slow_analyzer_cannot_block_capture() {
        let (mut meter, _receiver) = Meter::channel();
        meter.observe(&vec![0; 44100 * 4]);
        assert_eq!(meter.count, 0);
    }
    #[test]
    fn matches_original_tv_analysis_reference() {
        // Synthetic PCM fixture, with the original NumPy TV FFT as reference.
        let bytes = include_bytes!("../tests/fixtures/spectrum-reference.pcm");
        let samples: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| {
                [
                    f64::from(i16::from_be_bytes([b[0], b[1]])) / 32768.,
                    f64::from(i16::from_be_bytes([b[2], b[3]])) / 32768.,
                ]
            })
            .collect();
        let reference: [f64; 20] =
            serde_json::from_str(include_str!("../tests/fixtures/spectrum-reference.json"))
                .unwrap();
        for (band, (actual, expected)) in spectrum(&samples).iter().zip(reference).enumerate() {
            assert!(
                (actual - expected).abs() < 0.00001,
                "band {band}: {actual} != {expected}"
            );
        }
    }
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
            11
        );
        assert!((bands[11] + 9.03).abs() < 0.2, "{bands:?}");
        assert!(spectrum(&vec![[0.; 2]; N]).iter().all(|v| *v == -120.));
    }
}
