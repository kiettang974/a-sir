//! Any common audio or video file → 16 kHz mono f32, the only form the engine accepts.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
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

/// Disk-backed 16 kHz mono signed 16-bit little-endian PCM stream.
/// Lets long recordings (3–6+ hours) transcribe one 15-minute chunk at a time without
/// holding the full decoded audio in RAM.
#[derive(Debug, Clone)]
pub struct Pcm16File {
    path: PathBuf,
    len: usize,
}

impl Pcm16File {
    /// Opens an already-decoded PCM16 file if present and non-empty.
    pub fn open_existing(path: &Path) -> Option<Self> {
        let bytes = std::fs::metadata(path).ok()?.len() as usize;
        if bytes >= 2 && bytes.is_multiple_of(2) {
            Some(Self {
                path: path.to_owned(),
                len: bytes / 2,
            })
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Reads `[start..end)` samples as f32 in `[-1.0, 1.0]`.
    pub fn read_range(&self, start: usize, end: usize) -> Result<Vec<f32>, String> {
        let end = end.min(self.len);
        let start = start.min(end);
        let count = end - start;
        if count == 0 {
            return Ok(Vec::new());
        }
        let mut file = File::open(&self.path).map_err(|e| format!("không đọc được bộ đệm audio: {e}"))?;
        file.seek(SeekFrom::Start((start * 2) as u64))
            .map_err(|e| format!("lỗi tua bộ đệm audio: {e}"))?;
        let mut raw = vec![0u8; count * 2];
        file.read_exact(&mut raw)
            .map_err(|e| format!("lỗi đọc đoạn audio: {e}"))?;
        let mut out = Vec::with_capacity(count);
        for &pair in raw.as_chunks::<2>().0 {
            let s = i16::from_le_bytes(pair);
            out.push(f32::from(s) / 32768.0);
        }
        Ok(out)
    }

    /// Sample indices where each silence of at least 0.5 s below −35 dB ends, computed in O(1) RAM.
    pub fn silence_ends(&self) -> Result<Vec<usize>, String> {
        const FRAME: usize = SAMPLE_RATE / 50; // 320 samples (20 ms)
        const MIN_FRAMES: usize = 25;
        let threshold = 10f32.powf(-35.0 / 20.0);
        let file = File::open(&self.path).map_err(|e| format!("không đọc được bộ đệm audio: {e}"))?;
        let mut reader = BufReader::with_capacity(64 * 1024, file);
        let mut buf = [0u8; FRAME * 2];
        let mut ends = Vec::new();
        let mut quiet = 0usize;
        let mut frame_idx = 0usize;
        loop {
            let mut filled = 0;
            while filled < buf.len() {
                let n = reader
                    .read(&mut buf[filled..])
                    .map_err(|e| format!("lỗi quét im lặng: {e}"))?;
                if n == 0 {
                    break;
                }
                filled += n;
            }
            let sample_count = filled / 2;
            if sample_count == 0 {
                break;
            }
            let sum_sq: f32 = buf[..sample_count * 2]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&p| {
                    let s = f32::from(i16::from_le_bytes(p)) / 32768.0;
                    s * s
                })
                .sum();
            let rms = (sum_sq / sample_count as f32).sqrt();
            if rms < threshold {
                quiet += 1;
            } else {
                if quiet >= MIN_FRAMES {
                    ends.push(frame_idx * FRAME);
                }
                quiet = 0;
            }
            frame_idx += 1;
            if filled < buf.len() {
                break;
            }
        }
        Ok(ends)
    }
}

/// Streams any supported audio/video file directly into a 16 kHz mono PCM16 file on disk
/// with bounded RAM usage (~a few MB regardless of recording length).
pub fn decode_to_pcm16(source: &Path, dest: &Path) -> Result<Pcm16File, String> {
    decode_native_to_pcm16(source, dest).or_else(|error| {
        decode_with_afconvert_to_pcm16(source, dest).ok_or(error)
    })
}

fn decode_with_afconvert_to_pcm16(source: &Path, dest: &Path) -> Option<Pcm16File> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let wav = dir.path().join("converted.wav");
    let converted = Command::new("afconvert")
        .args(["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"])
        .arg(source)
        .arg(&wav)
        .stderr(Stdio::null())
        .status()
        .ok()?
        .success();
    if converted {
        decode_native_to_pcm16(&wav, dest).ok()
    } else {
        None
    }
}

fn write_pcm16_samples(
    writer: &mut BufWriter<File>,
    samples: &[f32],
    skip_delay: &mut usize,
    written: &mut usize,
    limit: Option<usize>,
) -> Result<(), String> {
    for &s in samples {
        if *skip_delay > 0 {
            *skip_delay -= 1;
            continue;
        }
        if let Some(max) = limit
            && *written >= max
        {
            break;
        }
        let clamped = (s * 32767.0).clamp(-32768.0, 32767.0) as i16;
        writer
            .write_all(&clamped.to_le_bytes())
            .map_err(|e| format!("lỗi ghi bộ đệm audio: {e}"))?;
        *written += 1;
    }
    Ok(())
}

fn decode_native_to_pcm16(source: &Path, dest: &Path) -> Result<Pcm16File, String> {
    let file = File::open(source).map_err(|e| format!("không mở được file: {e}"))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = source.extension().and_then(|e| e.to_str()) {
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

    let out_file = File::create(dest).map_err(|e| format!("không tạo được bộ đệm audio: {e}"))?;
    let mut writer = BufWriter::with_capacity(128 * 1024, out_file);
    let fail_rs = |e: &dyn std::fmt::Display| format!("lỗi đổi tần số mẫu: {e}");

    let mut rate = track.codec_params.sample_rate.unwrap_or(0) as usize;
    let mut resampler: Option<FftFixedIn<f32>> = None;
    let mut pending: Vec<f32> = Vec::with_capacity(8192);
    let mut skip_delay = 0usize;
    let mut total_in = 0usize;
    let mut written = 0usize;
    let mut buffer: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
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
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(format!("lỗi giải mã: {e}")),
        };
        let spec = *decoded.spec();
        rate = spec.rate as usize;
        let channels = spec.channels.count().max(1);
        let buf = buffer.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        buf.copy_interleaved_ref(decoded);
        let chunk_start = pending.len();
        pending.extend(
            buf.samples()
                .chunks_exact(channels)
                .map(|f| f.iter().sum::<f32>() / channels as f32),
        );
        total_in += pending.len() - chunk_start;

        if rate == SAMPLE_RATE {
            write_pcm16_samples(&mut writer, &pending, &mut skip_delay, &mut written, None)?;
            pending.clear();
        } else {
            let rs = match resampler.as_mut() {
                Some(r) => r,
                None => {
                    let r = FftFixedIn::<f32>::new(rate, SAMPLE_RATE, 4096, 2, 1)
                        .map_err(|e| fail_rs(&e))?;
                    skip_delay = r.output_delay();
                    resampler.insert(r)
                }
            };
            while pending.len() >= rs.input_frames_next() {
                let need = rs.input_frames_next();
                let resampled = rs.process(&[&pending[..need]], None).map_err(|e| fail_rs(&e))?;
                write_pcm16_samples(&mut writer, &resampled[0], &mut skip_delay, &mut written, None)?;
                pending.drain(..need);
            }
        }
    }

    if total_in == 0 {
        let _ = std::fs::remove_file(dest);
        return Err("file không có âm thanh".into());
    }

    if let Some(rs) = resampler.as_mut() {
        let expected = total_in * SAMPLE_RATE / rate;
        if !pending.is_empty() {
            let tail = rs
                .process_partial(Some(&[&pending[..]]), None)
                .map_err(|e| fail_rs(&e))?;
            write_pcm16_samples(&mut writer, &tail[0], &mut skip_delay, &mut written, Some(expected))?;
        }
        let flushed = rs
            .process_partial::<&[f32]>(None, None)
            .map_err(|e| fail_rs(&e))?;
        write_pcm16_samples(&mut writer, &flushed[0], &mut skip_delay, &mut written, Some(expected))?;
    }

    writer.flush().map_err(|e| format!("lỗi ghi bộ đệm audio: {e}"))?;
    Ok(Pcm16File {
        path: dest.to_owned(),
        len: written,
    })
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

    #[test]
    fn streaming_pcm16_decode_matches_in_memory_length_pitch_and_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("tone.wav");
        let pcm_path = dir.path().join("audio_16k.pcm");
        write_tone(&src, 44_100, 2, 3.0);
        let pcm = decode_to_pcm16(&src, &pcm_path).unwrap();
        assert_eq!(pcm.len(), 48_000);
        let slice = pcm.read_range(8_000, 40_000).unwrap();
        assert_eq!(slice.len(), 32_000);
        assert!((peak_hz(&slice) - 440.0).abs() < 3.0, "{}", peak_hz(&slice));
        assert_eq!(Pcm16File::open_existing(&pcm_path).unwrap().len(), 48_000);
    }
}
