//! Engine benchmark: transcribe a 16 kHz mono WAV and report RTF, raw/normalized
//! Vietnamese WER & CER (keeping diacritics, Unicode NFC), and hallucination count.
//!
//! bench <model> <vad> <wav> <reference.txt> <out.txt> [--beam N] [--no-vad] [--pad MS] [--ts]

use std::path::Path;
use std::time::Instant;

use asr_core::output::to_txt;
use asr_core::transcriber::{Control, Transcriber, WhisperCpp, WhisperOptions};
use asr_core::transcript::lines;
use unicode_normalization::UnicodeNormalization;

fn extract_body(text: &str) -> String {
    let parsed = lines(text);
    if parsed.is_empty() {
        text.to_owned()
    } else {
        parsed
            .into_iter()
            .map(|l| l.text)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn raw_words(text: &str) -> Vec<String> {
    extract_body(text)
        .nfc()
        .collect::<String>()
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

/// Normalizes punctuation & case in Unicode NFC while preserving Vietnamese diacritics.
fn normalized_words(text: &str) -> Vec<String> {
    extract_body(text)
        .nfc()
        .collect::<String>()
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

fn normalized_chars(text: &str) -> Vec<char> {
    normalized_words(text).join(" ").chars().collect()
}

fn rate(dist: usize, ref_len: usize) -> f64 {
    if ref_len == 0 {
        0.0
    } else {
        100.0 * dist as f64 / ref_len as f64
    }
}

fn count_phantoms(text: &str) -> usize {
    let lower = text.to_lowercase();
    ["subscribe", "đăng ký kênh", "hãy đăng ký", "cảm ơn các bạn đã theo dõi"]
        .iter()
        .map(|p| lower.matches(p).count())
        .sum()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 6 {
        eprintln!("Usage: bench <model> <vad> <wav> <reference.txt> <out.txt> [--beam N] [--no-vad] [--pad MS] [--ts]");
        std::process::exit(1);
    }
    let flag = |name: &str| a.iter().position(|x| x == name);
    let options = WhisperOptions {
        beam_size: flag("--beam").map_or(1, |i| a[i + 1].parse().unwrap()),
        use_vad: flag("--no-vad").is_none(),
        timestamps: flag("--ts").is_some(),
        speech_pad_ms: flag("--pad").map_or(400, |i| a[i + 1].parse().unwrap()),
        ..Default::default()
    };

    let samples: Vec<f32> = hound::WavReader::open(&a[3])
        .unwrap()
        .into_samples::<i16>()
        .map(|s| s.unwrap() as f32 / 32768.0)
        .collect();
    let audio_s = samples.len() as f64 / 16000.0;

    let t = Instant::now();
    let engine = WhisperCpp::load(Path::new(&a[1]), Path::new(&a[2]), options.clone()).unwrap();
    let load_s = t.elapsed().as_secs_f64();

    let t = Instant::now();
    let segments = engine.transcribe(&samples, &Control::default()).unwrap();
    let run_s = t.elapsed().as_secs_f64();

    let out = to_txt(&segments);
    std::fs::write(&a[5], &out).unwrap();

    let ref_text = std::fs::read_to_string(&a[4]).unwrap();
    let ref_raw = raw_words(&ref_text);
    let hyp_raw = raw_words(&out);
    let wer_raw = rate(strsim::generic_levenshtein(&ref_raw, &hyp_raw), ref_raw.len());

    let ref_norm = normalized_words(&ref_text);
    let hyp_norm = normalized_words(&out);
    let wer_norm = rate(strsim::generic_levenshtein(&ref_norm, &hyp_norm), ref_norm.len());

    let ref_chars = normalized_chars(&ref_text);
    let hyp_chars = normalized_chars(&out);
    let cer_norm = rate(strsim::generic_levenshtein(&ref_chars, &hyp_chars), ref_chars.len());

    let rtf = if audio_s > 0.0 { run_s / audio_s } else { 0.0 };
    let phantom = count_phantoms(&out);

    println!(
        "{options:?}\n\
         audio {audio_s:.1}s | load {load_s:.2}s | run {run_s:.1}s | RTF {rtf:.3} ({:.1}x realtime) | {} segments\n\
         words {} vs ref {} | WER(raw) {wer_raw:.2}% | WER(NFC norm) {wer_norm:.2}% | CER(NFC norm) {cer_norm:.2}% | phantom {phantom}",
        audio_s / run_s.max(0.001),
        segments.len(),
        hyp_norm.len(),
        ref_norm.len(),
    );
}
