//! The seam between jobs and the speech engine. Input is always 16 kHz mono f32.

use std::ffi::c_void;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperVadContext,
    WhisperVadContextParams, WhisperVadParams,
};

use crate::glossary::Term;

pub const SAMPLE_RATE: usize = 16_000;
const MAX_CHUNK_S: usize = 30;
const MAX_PROMPT_CHARS: usize = 600;

/// Builds a concise prompt from the user's glossary (`correct` column) for Whisper
/// `initial_prompt` and Qwen3-ASR `set_prompt`, capped to stay well inside token limits.
pub fn glossary_prompt(terms: &[Term]) -> Option<String> {
    let mut out = String::new();
    let mut seen = std::collections::HashSet::new();
    for term in terms {
        let word = term.correct.trim();
        if word.is_empty() || !seen.insert(word.to_lowercase()) {
            continue;
        }
        let sep = if out.is_empty() { "Thuật ngữ: " } else { ", " };
        if out.chars().count() + sep.chars().count() + word.chars().count() > MAX_PROMPT_CHARS {
            break;
        }
        out.push_str(sep);
        out.push_str(word);
    }
    (!out.is_empty()).then_some(out)
}

/// Controls how many CPU threads local ASR engines use (Metal GPU stays on for Whisper).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceProfile {
    Eco,
    #[default]
    Balanced,
    Max,
}

impl ResourceProfile {
    pub fn available_cores() -> usize {
        std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(8)
            .max(1)
    }

    pub fn threads(self) -> usize {
        let cores = Self::available_cores();
        match self {
            ResourceProfile::Eco => (cores * 2 / 5).max(2).min(cores),
            ResourceProfile::Balanced => (cores * 7 / 10).max(2).min(cores),
            ResourceProfile::Max => cores,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, thiserror::Error)]
pub enum TranscribeError {
    #[error("đã huỷ")]
    Cancelled,
    #[error("lỗi nhận dạng: {0}")]
    Engine(String),
}

/// How a running transcription reports progress (0–100) and learns it must stop.
#[derive(Clone)]
pub struct Control {
    pub cancel: Arc<AtomicBool>,
    pub on_progress: Arc<dyn Fn(u8) + Send + Sync>,
}

impl Default for Control {
    fn default() -> Self {
        Self {
            cancel: Arc::default(),
            on_progress: Arc::new(|_| {}),
        }
    }
}

pub trait Transcriber {
    /// Timestamps are relative to `samples`. Returns `Cancelled` soon after `control.cancel` is set.
    fn transcribe(
        &self,
        samples: &[f32],
        control: &Control,
    ) -> Result<Vec<Segment>, TranscribeError>;
}

#[derive(Debug, Clone)]
pub struct WhisperOptions {
    pub language: String,
    pub beam_size: i32,
    pub use_vad: bool,
    pub speech_pad_ms: i32,
    /// Off = one line per chunk (about 30 s), as faster-whisper's batched pipeline does.
    pub timestamps: bool,
    pub n_threads: i32,
    pub initial_prompt: Option<String>,
}

impl Default for WhisperOptions {
    /// Same settings as the old worker: Vietnamese, beam 1, VAD with 500 ms minimum silence.
    fn default() -> Self {
        Self {
            language: "vi".into(),
            beam_size: 1,
            use_vad: true,
            speech_pad_ms: 400,
            timestamps: false,
            n_threads: ResourceProfile::default().threads() as i32,
            initial_prompt: None,
        }
    }
}

pub struct WhisperCpp {
    ctx: WhisperContext,
    vad_model: String,
    options: WhisperOptions,
}

impl WhisperCpp {
    pub fn load(
        model: &Path,
        vad_model: &Path,
        options: WhisperOptions,
    ) -> Result<Self, TranscribeError> {
        let engine = |e: whisper_rs::WhisperError| TranscribeError::Engine(e.to_string());
        let path = |p: &Path| {
            p.to_str()
                .map(str::to_owned)
                .ok_or_else(|| TranscribeError::Engine(format!("đường dẫn lỗi: {}", p.display())))
        };
        let ctx =
            WhisperContext::new_with_params(&path(model)?, WhisperContextParameters::default())
                .map_err(engine)?;
        Ok(Self {
            ctx,
            vad_model: path(vad_model)?,
            options,
        })
    }
}

unsafe extern "C" fn abort_requested(flag: *mut c_void) -> bool {
    unsafe { &*(flag as *const AtomicBool) }.load(Ordering::Relaxed)
}

/// whisper.cpp only honours `params.vad` in `whisper_full`; whisper-rs calls
/// `whisper_full_with_state`, which ignores it. Shared by both WhisperCpp and QwenAsr.
fn vad_speech_spans(
    vad_model: &str,
    samples: &[f32],
    speech_pad_ms: i32,
) -> Result<Vec<(usize, usize)>, TranscribeError> {
    let engine = |e: whisper_rs::WhisperError| TranscribeError::Engine(e.to_string());
    let mut ctx = WhisperVadContext::new(vad_model, WhisperVadContextParams::default())
        .map_err(engine)?;
    // Same values faster-whisper's batched pipeline runs Silero with.
    let mut vad = WhisperVadParams::new();
    vad.set_min_speech_duration(0);
    vad.set_min_silence_duration(500);
    vad.set_max_speech_duration(MAX_CHUNK_S as f32);
    vad.set_speech_pad(speech_pad_ms);
    let per_cs = SAMPLE_RATE / 100;
    let spans = ctx
        .segments_from_samples(vad, samples)
        .map_err(engine)?
        .map(|s| {
            let end = (s.end as usize * per_cs).min(samples.len());
            ((s.start as usize * per_cs).min(end), end)
        })
        .collect::<Vec<_>>();
    Ok(merge_spans(&spans, MAX_CHUNK_S * SAMPLE_RATE))
}

impl WhisperCpp {
    fn speech_spans(&self, samples: &[f32]) -> Result<Vec<(usize, usize)>, TranscribeError> {
        if !self.options.use_vad {
            return Ok(vec![(0, samples.len())]);
        }
        vad_speech_spans(&self.vad_model, samples, self.options.speech_pad_ms)
    }
}

/// Joins consecutive speech spans while the result still fits one Whisper window.
fn merge_spans(spans: &[(usize, usize)], max_len: usize) -> Vec<(usize, usize)> {
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for &(start, end) in spans {
        match merged.last_mut() {
            Some(last) if end - last.0 <= max_len => last.1 = end,
            _ => merged.push((start, end)),
        }
    }
    merged
}

impl Transcriber for WhisperCpp {
    fn transcribe(
        &self,
        samples: &[f32],
        control: &Control,
    ) -> Result<Vec<Segment>, TranscribeError> {
        let engine = |e: whisper_rs::WhisperError| TranscribeError::Engine(e.to_string());
        let mut state = self.ctx.create_state().map_err(engine)?;
        let spans = self.speech_spans(samples)?;
        let total: usize = spans.iter().map(|(s, e)| e - s).sum::<usize>().max(1);
        let mut finished = 0;
        let mut segments = Vec::new();

        for &(start, end) in &spans {
            if control.cancel.load(Ordering::Relaxed) {
                return Err(TranscribeError::Cancelled);
            }
            let strategy = if self.options.beam_size > 1 {
                SamplingStrategy::BeamSearch {
                    beam_size: self.options.beam_size,
                    patience: -1.0,
                }
            } else {
                SamplingStrategy::Greedy { best_of: 1 }
            };
            let mut params = FullParams::new(strategy);
            params.set_language(Some(&self.options.language));
            params.set_n_threads(self.options.n_threads.max(1));
            if let Some(prompt) = self
                .options
                .initial_prompt
                .as_deref()
                .filter(|s| !s.is_empty())
            {
                params.set_initial_prompt(prompt);
            }
            // Like faster-whisper's batched pipeline: every chunk decodes on its own.
            params.set_no_context(true);
            params.set_no_timestamps(!self.options.timestamps);
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            // Not set_abort_callback_safe: whisper-rs 0.16 boxes the closure as `dyn FnMut` but its
            // trampoline reads it as the concrete type, so the callback sees garbage and aborts at once.
            unsafe {
                params.set_abort_callback(Some(abort_requested));
                // `control` outlives the state.full() call below, the only place this is read.
                params.set_abort_callback_user_data(Arc::as_ptr(&control.cancel) as *mut c_void);
            }

            let result = state.full(params, &samples[start..end]);
            if control.cancel.load(Ordering::Relaxed) {
                return Err(TranscribeError::Cancelled);
            }
            result.map_err(engine)?;

            let to_ms = |n: usize| (n * 1000 / SAMPLE_RATE) as u64;
            for s in state.as_iter() {
                let text = s.to_str_lossy().map_err(engine)?.trim().to_owned();
                if text.is_empty() {
                    continue;
                }
                let (from, to) = if self.options.timestamps {
                    // whisper.cpp timestamps are in centiseconds
                    (
                        s.start_timestamp().max(0) as u64 * 10,
                        s.end_timestamp().max(0) as u64 * 10,
                    )
                } else {
                    (0, to_ms(end - start))
                };
                segments.push(Segment {
                    start_ms: to_ms(start) + from,
                    end_ms: (to_ms(start) + to).min(to_ms(end)),
                    text,
                });
            }
            finished += end - start;
            (control.on_progress)((finished * 100 / total) as u8);
        }
        Ok(segments)
    }
}

/// Qwen3-ASR relies on POSIX `mmap`, so Windows builds ship without it.
#[cfg(not(unix))]
pub struct QwenAsr;

#[cfg(not(unix))]
impl QwenAsr {
    pub fn load(
        _model_dir: &Path,
        _vad_model: &Path,
        _threads: usize,
        _prompt: Option<String>,
    ) -> Result<Self, TranscribeError> {
        Err(TranscribeError::Engine(
            "Qwen3-ASR chưa chạy được trên Windows, hãy chọn model Whisper hoặc PhoWhisper.".into(),
        ))
    }
}

#[cfg(not(unix))]
impl Transcriber for QwenAsr {
    fn transcribe(&self, _: &[f32], _: &Control) -> Result<Vec<Segment>, TranscribeError> {
        Err(TranscribeError::Engine("Qwen3-ASR chưa chạy được trên Windows.".into()))
    }
}

/// Pure-Rust Qwen3-ASR adapter using `qwen-asr` (Accelerate/vDSP on macOS) + shared Silero VAD
/// for segment timestamps and responsive cancellation.
#[cfg(unix)]
pub struct QwenAsr {
    model: Arc<qwen_asr::context::QwenModel>,
    vad_model: String,
    language: String,
    threads: usize,
    prompt: Option<String>,
}

#[cfg(unix)]
impl QwenAsr {
    pub fn load(
        model_dir: &Path,
        vad_model: &Path,
        threads: usize,
        prompt: Option<String>,
    ) -> Result<Self, TranscribeError> {
        let to_str = |p: &Path| {
            p.to_str()
                .map(str::to_owned)
                .ok_or_else(|| TranscribeError::Engine(format!("đường dẫn lỗi: {}", p.display())))
        };
        let dir_str = to_str(model_dir)?;
        let vad_str = to_str(vad_model)?;
        let threads = threads.max(1);
        qwen_asr::kernels::set_threads(threads);
        let model = qwen_asr::context::QwenModel::load(&dir_str).ok_or_else(|| {
            TranscribeError::Engine(format!("không nạp được model Qwen3-ASR tại {dir_str}"))
        })?;
        Ok(Self {
            model,
            vad_model: vad_str,
            language: "Vietnamese".into(),
            threads,
            prompt,
        })
    }
}

#[cfg(unix)]
impl Transcriber for QwenAsr {
    fn transcribe(
        &self,
        samples: &[f32],
        control: &Control,
    ) -> Result<Vec<Segment>, TranscribeError> {
        qwen_asr::kernels::set_threads(self.threads);
        let spans = vad_speech_spans(&self.vad_model, samples, 400)?;
        let total: usize = spans.iter().map(|(s, e)| e - s).sum::<usize>().max(1);
        let mut finished = 0;
        let mut segments = Vec::new();
        let mut ctx = self.model.new_session();
        let _ = ctx.set_force_language(&self.language);
        if let Some(prompt) = self.prompt.as_deref().filter(|s| !s.is_empty()) {
            let _ = ctx.set_prompt(prompt);
        }

        let to_ms = |n: usize| (n * 1000 / SAMPLE_RATE) as u64;
        for &(start, end) in &spans {
            if control.cancel.load(Ordering::Relaxed) {
                return Err(TranscribeError::Cancelled);
            }
            let raw = qwen_asr::transcribe::transcribe_audio(&mut ctx, &samples[start..end])
                .ok_or_else(|| {
                    TranscribeError::Engine("Qwen3-ASR không trả về kết quả nhận dạng".into())
                })?;
            if control.cancel.load(Ordering::Relaxed) {
                return Err(TranscribeError::Cancelled);
            }
            let text = raw
                .rsplit_once("<asr_text>")
                .map_or(raw.as_str(), |(_, tail)| tail)
                .trim()
                .to_owned();
            if !text.is_empty() {
                segments.push(Segment {
                    start_ms: to_ms(start),
                    end_ms: to_ms(end),
                    text,
                });
            }
            finished += end - start;
            (control.on_progress)((finished * 100 / total) as u8);
        }
        Ok(segments)
    }
}

#[cfg(test)]
mod tests {
    use super::{glossary_prompt, merge_spans};
    use crate::glossary::Term;

    #[test]
    fn merges_until_the_window_is_full() {
        let spans = [(0, 10), (12, 25), (28, 40), (41, 45)];
        assert_eq!(merge_spans(&spans, 30), [(0, 25), (28, 45)]);
    }

    #[test]
    fn keeps_a_span_longer_than_the_window_whole() {
        assert_eq!(merge_spans(&[(0, 50), (51, 60)], 30), [(0, 50), (51, 60)]);
    }

    #[test]
    fn nothing_in_nothing_out() {
        assert!(merge_spans(&[], 30).is_empty());
    }

    #[test]
    fn glossary_prompt_deduplicates_skips_blanks_and_caps_length() {
        assert_eq!(glossary_prompt(&[]), None);
        let terms = vec![
            Term {
                correct: "   ".into(),
                heard_as: String::new(),
                note: String::new(),
            },
            Term {
                correct: "VietQR".into(),
                heard_as: "việt quy a".into(),
                note: String::new(),
            },
            Term {
                correct: "vietqr".into(),
                heard_as: String::new(),
                note: String::new(),
            },
            Term {
                correct: "VNPay".into(),
                heard_as: String::new(),
                note: String::new(),
            },
        ];
        assert_eq!(glossary_prompt(&terms).as_deref(), Some("Thuật ngữ: VietQR, VNPay"));
    }
}
