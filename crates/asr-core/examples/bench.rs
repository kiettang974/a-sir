//! Engine gate: transcribe a 16 kHz mono WAV, report speed and word-level
//! similarity against the transcript the old faster-whisper worker produced.
//! The reference is not ground truth: it only shows how far the two engines diverge.
//!
//! bench <model> <vad> <wav> <reference.txt> <out.txt> [--beam N] [--no-vad] [--pad MS] [--ts]

use std::path::Path;
use std::time::Instant;

use asr_core::output::to_txt;
use asr_core::transcriber::{Control, Transcriber, WhisperCpp, WhisperOptions};
use asr_core::transcript::lines;

fn words(text: &str) -> Vec<String> {
    lines(text)
        .iter()
        .flat_map(|l| l.text.split_whitespace())
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
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

    let reference = words(&std::fs::read_to_string(&a[4]).unwrap());
    let ours = words(&out);
    let dist = strsim::generic_levenshtein(&reference, &ours);
    let phantom = out.matches("subscribe").count();
    println!(
        "{options:?}\naudio {audio_s:.0}s | load {load_s:.1}s | run {run_s:.0}s | {:.1}x realtime | {} segments | words {} vs ref {} | word divergence {:.1}% | phantom lines {phantom}",
        audio_s / run_s,
        segments.len(),
        ours.len(),
        reference.len(),
        100.0 * dist as f64 / reference.len() as f64,
    );
}
