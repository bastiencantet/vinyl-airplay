//! Separate SongRec-based fingerprint helper. stdin: PCM S16BE/44100/stereo.
#![allow(clippy::needless_range_loop)]
#[allow(dead_code, clippy::all)]
mod vendor {
    pub mod algorithm;
    pub mod hanning;
    pub mod signature_format;
}
use std::io::{Read, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(44_100 * 4 * 12 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 44_100 * 4 * 12
        || bytes.len() < 44_100 * 4 * 6
        || !bytes.len().is_multiple_of(4)
    {
        return Err("expected 6..12 seconds of PCM S16BE/44100/stereo".into());
    }
    let mono: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|frame| {
            (f32::from(i16::from_be_bytes([frame[0], frame[1]]))
                + f32::from(i16::from_be_bytes([frame[2], frame[3]])))
                / 65536.0
        })
        .collect();
    let samples = resample(&mono);
    let signature = vendor::algorithm::SignatureGenerator::make_signature_from_buffer(&samples);
    let result = serde_json::json!({"uri": signature.encode_to_uri()?, "samplems": samples.len() * 1000 / 16000});
    std::io::stdout().write_all(result.to_string().as_bytes())?;
    Ok(())
}

// Rational 44100 -> 16000 windowed-sinc low pass. Coefficients calculated once;
// ~9 million multiplies for a 12-second excerpt, off the audio thread.
fn resample(input: &[f32]) -> Vec<f32> {
    const TAPS: usize = 48;
    let filters: Vec<[f32; TAPS]> = (0..160)
        .map(|phase| {
            let mut filter = [0.0; TAPS];
            for (tap, coefficient) in filter.iter_mut().enumerate() {
                let x = tap as f64 - 23.0 - phase as f64 / 160.0;
                let cutoff = 7200.0 / 44100.0;
                let sinc = if x.abs() < 1e-12 {
                    2.0 * cutoff
                } else {
                    (2.0 * std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x)
                };
                let window = 0.5 + 0.5 * (std::f64::consts::PI * x / 24.0).cos();
                *coefficient = (sinc * window) as f32;
            }
            let sum: f32 = filter.iter().sum();
            for coefficient in &mut filter {
                *coefficient /= sum;
            }
            filter
        })
        .collect();
    (0..input.len() * 160 / 441)
        .map(|index| {
            let center = index * 441 / 160;
            let phase = index * 441 % 160;
            filters[phase]
                .iter()
                .enumerate()
                .map(|(tap, coefficient)| {
                    let pos = center as isize + tap as isize - 23;
                    input.get(pos as usize).copied().unwrap_or(0.0) * coefficient
                })
                .sum()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resampling_preserves_music_band_and_rejects_aliases() {
        for (frequency, expected) in [(1000.0, true), (15000.0, false)] {
            let input: Vec<f32> = (0..44100)
                .map(|i| (i as f32 * frequency * std::f32::consts::TAU / 44100.0).sin())
                .collect();
            let output = resample(&input);
            assert_eq!(output.len(), 16000);
            let rms = (output[100..15900].iter().map(|x| x * x).sum::<f32>() / 15800.0).sqrt();
            if expected {
                assert!((rms - 0.707).abs() < 0.02);
            } else {
                assert!(rms < 0.01);
            }
        }
    }
}
