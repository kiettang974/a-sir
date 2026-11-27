//! Any common audio or video file → 16 kHz mono f32, the only form the engine accepts.

use std::fs::File;
use std::path::Path;
use std::process::{Command, Stdio};

use rubato::{FftFixedIn, Resampler};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::transcriber::SAMPLE_RATE;

/// File extensions `decode` is meant for: what Symphonia reads here, plus the QuickTime
/// containers macOS's own converter handles. The file picker and drop zone offer these.
pub const EXTENSIONS: [&str; 9] = ["wav", "m4a", "qta", "mp3", "mp4", "mov", "aac", "flac", "ogg"];

pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

pub fn decode(path: &Path) -> Result<Vec<f32>, String> {
    decode_native(path).or_else(|error| decode_with_afconvert(path).ok_or(error))
}

/// Voice Memos can store codecs Symphonia won't read inside QuickTime files, and macOS
/// always ships a converter that does.
fn decode_with_afconvert(path: &Path) -> Option<Vec<f32>> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let wav = dir.path().join("converted.wav");
    let converted = Command::new("afconvert")
        .args(["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"])
        .arg(path)
        .arg(&wav)
        .stderr(Stdio::null())
        .status()
        .ok()?
        .success();
    if converted {
        decode_native(&wav).ok()
    } else {
        None
    }
}

fn decode_native(path: &Path) -> Result<Vec<f32>, String> {
    let file = File::open(path).map_err(|e| format!("không mở được file: {e}"))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let unsupported = |e: SymphoniaError| format!("không đọc được định dạng này: {e}");
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(unsupported)?
        .format;
    let track = format
        .default_track()
        .ok_or("file không có luồng âm thanh")?
        .clone();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(unsupported)?;

    let mut rate = track.codec_params.sample_rate.unwrap_or(0) as usize;
    let mut mono = Vec::new();
    let mut buffer: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // Symphonia reports the end of a stream as an unexpected EOF.
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(e) => return Err(format!("lỗi đọc file: {e}")),
        };
        if packet.track_id() != track.id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // One damaged packet should not lose an hour-long recording.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(format!("lỗi giải mã: {e}")),
        };
        let spec = *decoded.spec();
        rate = spec.rate as usize;
        let channels = spec.channels.count();
        let buf = buffer.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        buf.copy_interleaved_ref(decoded);
        mono.extend(
            buf.samples()
                .chunks_exact(channels)
                .map(|f| f.iter().sum::<f32>() / channels as f32),
        );
    }
    if mono.is_empty() {
        return Err("file không có âm thanh".into());
    }
    resample(mono, rate)
}

fn resample(input: Vec<f32>, rate: usize) -> Result<Vec<f32>, String> {
    if rate == SAMPLE_RATE {
        return Ok(input);
    }
    let fail = |e: &dyn std::fmt::Display| format!("lỗi đổi tần số mẫu: {e}");
    let mut resampler =
        FftFixedIn::<f32>::new(rate, SAMPLE_RATE, 4096, 2, 1).map_err(|e| fail(&e))?;
    let expected = input.len() * SAMPLE_RATE / rate;
    let mut out = Vec::with_capacity(expected + 8192);
    let mut rest = &input[..];
    while rest.len() >= resampler.input_frames_next() {
        let (head, tail) = rest.split_at(resampler.input_frames_next());
        out.extend_from_slice(&resampler.process(&[head], None).map_err(|e| fail(&e))?[0]);
        rest = tail;
    }
    // An empty tail (length an exact multiple of the chunk) makes rubato reject the buffer.
    if !rest.is_empty() {
        out.extend_from_slice(
            &resampler
                .process_partial(Some(&[rest]), None)
                .map_err(|e| fail(&e))?[0],
        );
    }
    // Flush what the filter still holds; its delay is dropped so timestamps stay right.
    out.extend_from_slice(
        &resampler
            .process_partial::<&[f32]>(None, None)
            .map_err(|e| fail(&e))?[0],
    );
    let delay = resampler.output_delay();
    Ok(out.into_iter().skip(delay).take(expected).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tone(path: &Path, rate: u32, channels: u16, seconds: f32) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..(rate as f32 * seconds) as u32 {
            let v = (i as f32 / rate as f32 * 440.0 * std::f32::consts::TAU).sin() * 0.5;
            for _ in 0..channels {
                w.write_sample((v * 32767.0) as i16).unwrap();
            }
        }
        w.finalize().unwrap();
    }

    fn peak_hz(samples: &[f32]) -> f32 {
        let crossings = samples
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        crossings as f32 / (samples.len() as f32 / SAMPLE_RATE as f32)
    }

    #[test]
    fn stereo_44k_becomes_16k_mono_with_same_pitch_and_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.wav");
        write_tone(&path, 44_100, 2, 3.0);
        let out = decode(&path).unwrap();
        assert_eq!(out.len(), 48_000);
        assert!((peak_hz(&out) - 440.0).abs() < 3.0, "{}", peak_hz(&out));
    }

    #[test]
    fn input_length_that_is_a_multiple_of_the_chunk_resamples() {
        let out = resample(vec![0.0; 4096 * 43], 44_100).unwrap();
        assert_eq!(out.len(), 4096 * 43 * SAMPLE_RATE / 44_100);
    }

    #[test]
    fn already_16k_mono_keeps_every_sample() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.wav");
        write_tone(&path, 16_000, 1, 1.0);
        assert_eq!(decode(&path).unwrap().len(), 16_000);
    }

    #[test]
    fn a_file_that_is_not_audio_gives_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.wav");
        std::fs::write(&path, "chỉ là chữ, không phải âm thanh").unwrap();
        assert!(decode(&path).is_err());
    }
}
