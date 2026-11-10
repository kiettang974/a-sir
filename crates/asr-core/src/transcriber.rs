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

pub const SAMPLE_RATE: usize = 16_000;
const MAX_CHUNK_S: usize = 30;

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

impl WhisperCpp {
    /// whisper.cpp only honours `params.vad` in `whisper_full`; whisper-rs calls
    /// `whisper_full_with_state`, which ignores it. So VAD runs here instead.
    fn speech_spans(&self, samples: &[f32]) -> Result<Vec<(usize, usize)>, TranscribeError> {
        let engine = |e: whisper_rs::WhisperError| TranscribeError::Engine(e.to_string());
        if !self.options.use_vad {
            return Ok(vec![(0, samples.len())]);
        }
        let mut ctx = WhisperVadContext::new(&self.vad_model, WhisperVadContextParams::default())
            .map_err(engine)?;
        // Same values faster-whisper's batched pipeline runs Silero with.
        let mut vad = WhisperVadParams::new();
        vad.set_min_speech_duration(0);
        vad.set_min_silence_duration(500);
        vad.set_max_speech_duration(MAX_CHUNK_S as f32);
        vad.set_speech_pad(self.options.speech_pad_ms);
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
            params.set_n_threads(8);
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

#[cfg(test)]
mod tests {
    use super::merge_spans;

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
}
