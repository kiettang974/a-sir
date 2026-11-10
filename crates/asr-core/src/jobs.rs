//! The queue. Callers add files and watch `Job`s change; everything else lives here:
//! one job at a time, splitting long audio at silences, resuming after a crash,
//! pause, cancel, and writing the transcripts.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::output::{candidates, save_atomic};
use crate::transcriber::{Control, SAMPLE_RATE, Segment, TranscribeError, Transcriber};
use crate::transcript::Transcript;
use crate::{audio, output};

const CHUNK_S: usize = 900;
const SEARCH_S: usize = 120;
/// Only split when more than this remains, so the last piece is never tiny.
const MAX_LAST_CHUNK_S: usize = 1350;
const MIN_CHUNK_S: usize = 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum JobState {
    Queued,
    Running { percent: u8, eta_s: Option<u64> },
    Done { txt: PathBuf, srt: PathBuf },
    Failed { message: String },
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: u64,
    pub source: PathBuf,
    /// Names the transcript instead of the source file, whose own name may mean nothing.
    #[serde(default)]
    pub title: Option<String>,
    pub state: JobState,
}

/// What a watcher sees after every change.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Queue<'a> {
    pub jobs: &'a [Job],
    pub paused: bool,
    /// Work is waiting or running and the queue isn't paused: keep the machine awake.
    pub busy: bool,
}

impl<'a> Queue<'a> {
    fn of(state: &'a State) -> Self {
        Queue {
            jobs: &state.jobs,
            paused: state.paused,
            busy: !state.paused && state.jobs.iter().any(|j| j.state.is_active()),
        }
    }
}

impl JobState {
    /// Queued or running: work that is still to be done.
    pub fn is_active(&self) -> bool {
        matches!(self, JobState::Queued | JobState::Running { .. })
    }
}

/// Why a file was not added.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Rejected {
    #[error("file này đang chờ hoặc đang chạy (việc số {0})")]
    AlreadyQueued(u64),
    #[error("không đọc được định dạng file này")]
    Unsupported,
}

#[derive(Debug, Clone)]
pub struct Paths {
    /// App-private: the job list and checkpoints.
    pub data: PathBuf,
    /// Where transcripts land.
    pub output: PathBuf,
}

/// Finished chunks of one job, so a crash or a restart loses at most one chunk.
#[derive(Serialize, Deserialize)]
struct Checkpoint {
    fingerprint: String,
    chunks: Vec<(usize, usize)>,
    done: Vec<Vec<Segment>>,
}

struct State {
    jobs: Vec<Job>,
    paused: bool,
    next_id: u64,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    /// Interrupts the chunk being transcribed, for pause or for cancelling the current job.
    interrupt: Arc<AtomicBool>,
    current: Mutex<Option<u64>>,
    paths: Paths,
    on_change: Box<dyn Fn(&Queue) + Send + Sync>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Saves the job list and tells the watcher.
    fn publish(&self, state: &State) {
        let saved = save_atomic(
            &self.paths.data.join("jobs.json"),
            &serde_json::to_vec(&state.jobs).unwrap_or_default(),
        );
        if let Err(e) = saved {
            eprintln!("không lưu được danh sách việc: {e}");
        }
        self.notify(state);
    }

    /// Tells the watcher without saving. For progress ticks: a running job is queued again
    /// on restart, so its percentage on disk is never read.
    fn notify(&self, state: &State) {
        (self.on_change)(&Queue::of(state));
    }

    fn set_state(&self, id: u64, new: JobState) {
        let mut state = self.lock();
        if let Some(job) = state.jobs.iter_mut().find(|j| j.id == id) {
            job.state = new;
        }
        self.publish(&state);
    }

    fn work_dir(&self, id: u64) -> PathBuf {
        self.paths.data.join("work").join(id.to_string())
    }
}

pub struct JobRunner {
    shared: Arc<Shared>,
}

impl JobRunner {
    /// Reloads earlier jobs and starts the worker. Jobs that were running when the app last
    /// closed are queued again. `load` runs when a job needs an engine and none is loaded: the
    /// engine is dropped whenever the queue runs empty, and a failed load is retried on the next
    /// job. `on_change` runs with the queue locked, so it must be quick and must not call back
    /// into this runner.
    pub fn start<L, C>(paths: Paths, load: L, on_change: C) -> std::io::Result<Self>
    where
        L: Fn() -> Result<Box<dyn Transcriber>, TranscribeError> + Send + 'static,
        C: Fn(&Queue) + Send + Sync + 'static,
    {
        fs::create_dir_all(&paths.data)?;
        fs::create_dir_all(&paths.output)?;
        let mut jobs: Vec<Job> = fs::read(paths.data.join("jobs.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        for job in &mut jobs {
            if matches!(job.state, JobState::Running { .. }) {
                job.state = JobState::Queued;
            }
        }
        let next_id = jobs.iter().map(|j| j.id + 1).max().unwrap_or(1);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                jobs,
                paused: false,
                next_id,
            }),
            wake: Condvar::new(),
            interrupt: Arc::default(),
            current: Mutex::default(),
            paths,
            on_change: Box::new(on_change),
        });
        let worker = shared.clone();
        thread::spawn(move || run_worker(&worker, load));
        Ok(Self { shared })
    }

    pub fn jobs(&self) -> Vec<Job> {
        self.shared.lock().jobs.clone()
    }

    pub fn is_paused(&self) -> bool {
        self.shared.lock().paused
    }

    pub fn is_busy(&self) -> bool {
        Queue::of(&self.shared.lock()).busy
    }

    pub fn enqueue(&self, source: PathBuf) -> Result<u64, Rejected> {
        self.enqueue_titled(source, None)
    }

    /// Refuses a file whose format can't be read, and a file that is already waiting or
    /// running. A finished, failed or cancelled file may be added again.
    pub fn enqueue_titled(&self, source: PathBuf, title: Option<String>) -> Result<u64, Rejected> {
        if !audio::is_supported(&source) {
            return Err(Rejected::Unsupported);
        }
        let mut state = self.shared.lock();
        if let Some(job) = state
            .jobs
            .iter()
            .find(|j| j.source == source && j.state.is_active())
        {
            return Err(Rejected::AlreadyQueued(job.id));
        }
        let id = state.next_id;
        state.next_id += 1;
        state.jobs.push(Job {
            id,
            source,
            title,
            state: JobState::Queued,
        });
        self.shared.publish(&state);
        self.shared.wake.notify_all();
        Ok(id)
    }

    /// The transcript of a finished job.
    pub fn transcript(&self, id: u64) -> Result<Transcript, String> {
        self.shared
            .lock()
            .jobs
            .iter()
            .find_map(|j| match &j.state {
                JobState::Done { txt, .. } if j.id == id => {
                    Some(Transcript::new(txt.clone(), j.source.clone()))
                }
                _ => None,
            })
            .ok_or_else(|| "bản chép lời này không còn trong danh sách".to_owned())
    }

    /// Takes effect at once: the chunk in progress stops and is redone on resume.
    pub fn set_paused(&self, paused: bool) {
        let mut state = self.shared.lock();
        state.paused = paused;
        self.shared.interrupt.store(paused, Ordering::Relaxed);
        self.shared.publish(&state);
        self.shared.wake.notify_all();
    }

    /// Stops a queued or running job and forgets its progress.
    pub fn cancel(&self, id: u64) {
        let mut state = self.shared.lock();
        let Some(job) = state.jobs.iter_mut().find(|j| j.id == id) else {
            return;
        };
        if !matches!(job.state, JobState::Queued | JobState::Running { .. }) {
            return;
        }
        job.state = JobState::Cancelled;
        if *self
            .shared
            .current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            == Some(id)
        {
            self.shared.interrupt.store(true, Ordering::Relaxed);
        } else {
            fs::remove_dir_all(self.shared.work_dir(id)).ok();
        }
        self.shared.publish(&state);
        self.shared.wake.notify_all();
    }

    /// Queues a failed or cancelled job again. A failed job keeps its finished chunks.
    pub fn retry(&self, id: u64) {
        let mut state = self.shared.lock();
        if let Some(job) = state.jobs.iter_mut().find(|j| j.id == id)
            && matches!(job.state, JobState::Failed { .. } | JobState::Cancelled)
        {
            job.state = JobState::Queued;
            self.shared.publish(&state);
            self.shared.wake.notify_all();
        }
    }

    /// Removes a job that isn't running from the list. Transcripts stay on disk.
    pub fn remove(&self, id: u64) {
        let mut state = self.shared.lock();
        let before = state.jobs.len();
        state
            .jobs
            .retain(|j| j.id != id || matches!(j.state, JobState::Running { .. }));
        if state.jobs.len() != before {
            fs::remove_dir_all(self.shared.work_dir(id)).ok();
            self.shared.publish(&state);
        }
    }
}

fn run_worker<L>(shared: &Arc<Shared>, load: L)
where
    L: Fn() -> Result<Box<dyn Transcriber>, TranscribeError>,
{
    let mut engine: Option<Box<dyn Transcriber>> = None;
    loop {
        let job = {
            let mut state = shared.lock();
            loop {
                let next = state.jobs.iter().find(|j| j.state == JobState::Queued);
                match next.cloned() {
                    Some(job) if !state.paused => break job,
                    Some(_) => {}
                    // The model takes about 1.8 GB and reloads in about a second, so
                    // an idle app shouldn't hold on to it. A paused queue keeps it.
                    None => engine = None,
                }
                state = shared.wake.wait(state).unwrap_or_else(|e| e.into_inner());
            }
        };
        *shared.current.lock().unwrap_or_else(|e| e.into_inner()) = Some(job.id);
        shared.set_state(
            job.id,
            JobState::Running {
                percent: 0,
                eta_s: None,
            },
        );

        let outcome = match &engine {
            Some(_) => Ok(()),
            None => load().map(|e| engine = Some(e)).map_err(|e| e.to_string()),
        }
        .and_then(|()| process(shared, engine.as_deref().expect("loaded above"), &job));
        finish_current(shared);
        match outcome {
            Ok(done) => shared.set_state(job.id, done),
            Err(_) if is_cancelled(shared, job.id) => {}
            Err(message) => shared.set_state(job.id, JobState::Failed { message }),
        }
    }
}

fn finish_current(shared: &Shared) {
    *shared.current.lock().unwrap_or_else(|e| e.into_inner()) = None;
    // Only a pause may stay interrupting across jobs.
    let paused = shared.lock().paused;
    shared.interrupt.store(paused, Ordering::Relaxed);
}

fn is_cancelled(shared: &Shared, id: u64) -> bool {
    shared
        .lock()
        .jobs
        .iter()
        .find(|j| j.id == id)
        .is_none_or(|j| j.state == JobState::Cancelled)
}

fn process(shared: &Arc<Shared>, engine: &dyn Transcriber, job: &Job) -> Result<JobState, String> {
    let samples = audio::decode(&job.source)?;
    let fingerprint = fingerprint(&job.source)?;
    let work = shared.work_dir(job.id);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let checkpoint_path = work.join("checkpoint.json");

    let mut checkpoint = fs::read(&checkpoint_path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Checkpoint>(&b).ok())
        .filter(|c| c.fingerprint == fingerprint)
        .unwrap_or_else(|| Checkpoint {
            fingerprint,
            chunks: plan_chunks(&silence_ends(&samples), samples.len()),
            done: Vec::new(),
        });

    let total = samples.len().max(1);
    let started = Instant::now();
    let resumed: usize = checkpoint.chunks[..checkpoint.done.len()]
        .iter()
        .map(|(s, e)| e - s)
        .sum();
    let progress_of = {
        let chunks = checkpoint.chunks.clone();
        move |index: usize, percent: u8| {
            let before: usize = chunks[..index].iter().map(|(s, e)| e - s).sum();
            before + (chunks[index].1 - chunks[index].0) * percent as usize / 100
        }
    };

    while checkpoint.done.len() < checkpoint.chunks.len() {
        wait_while_paused(shared);
        if is_cancelled(shared, job.id) {
            fs::remove_dir_all(&work).ok();
            return Ok(JobState::Cancelled);
        }
        // Whatever interrupted the last attempt has been dealt with above.
        shared.interrupt.store(false, Ordering::Relaxed);
        let index = checkpoint.done.len();
        let (start, end) = checkpoint.chunks[index];
        let control = Control {
            cancel: shared.interrupt.clone(),
            on_progress: {
                let progress_of = progress_of.clone();
                let report = report_progress(shared.clone(), job.id, total, resumed, started);
                Arc::new(move |p| report(progress_of(index, p)))
            },
        };
        match engine.transcribe(&samples[start..end], &control) {
            Ok(mut segments) => {
                let offset = (start * 1000 / SAMPLE_RATE) as u64;
                for s in &mut segments {
                    s.start_ms += offset;
                    s.end_ms += offset;
                }
                checkpoint.done.push(segments);
                save_atomic(
                    &checkpoint_path,
                    &serde_json::to_vec(&checkpoint).map_err(|e| e.to_string())?,
                )
                .map_err(|e| format!("không lưu được tiến độ: {e}"))?;
                report_progress(shared.clone(), job.id, total, resumed, started)(progress_of(
                    index, 100,
                ));
            }
            // Pause or cancel: go round again, the checks at the top decide which.
            Err(TranscribeError::Cancelled) => continue,
            Err(e) => return Err(e.to_string()),
        }
    }

    let segments: Vec<Segment> = checkpoint.done.into_iter().flatten().collect();
    let (txt, srt) = write_outputs(&shared.paths.output, &output_name(job), &segments)
        .map_err(|e| format!("không ghi được file kết quả: {e}"))?;
    fs::remove_dir_all(&work).ok();
    Ok(JobState::Done { txt, srt })
}

fn wait_while_paused(shared: &Shared) {
    let mut state = shared.lock();
    while state.paused {
        state = shared.wake.wait(state).unwrap_or_else(|e| e.into_inner());
    }
}

/// Reports only on whole-percent changes, so the UI isn't flooded.
fn report_progress(
    shared: Arc<Shared>,
    id: u64,
    total: usize,
    resumed: usize,
    started: Instant,
) -> impl Fn(usize) + Send + Sync + 'static {
    move |done: usize| {
        let done_now = done.saturating_sub(resumed);
        let eta_s = (done_now > 0).then(|| {
            let rate = done_now as f64 / started.elapsed().as_secs_f64().max(0.001);
            ((total - done.min(total)) as f64 / rate) as u64
        });
        let percent = (done * 100 / total) as u8;
        let mut state = shared.lock();
        if let Some(job) = state.jobs.iter_mut().find(|j| j.id == id)
            && let JobState::Running {
                percent: old,
                eta_s: old_eta,
            } = &mut job.state
            && *old != percent
        {
            *old = percent;
            *old_eta = eta_s;
            shared.notify(&state);
        }
    }
}

fn fingerprint(path: &Path) -> Result<String, String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    Ok(format!("{}:{modified}", meta.len()))
}

/// A title like "Họp 09/11/2026" is user text, so it can't be used as a file name as it is.
fn output_name(job: &Job) -> String {
    let name = match &job.title {
        Some(title) => title
            .chars()
            .map(|c| {
                if matches!(c, '/' | ':' | '\\') || c.is_control() {
                    '-'
                } else {
                    c
                }
            })
            .collect::<String>()
            .trim()
            .trim_start_matches('.')
            .to_owned(),
        None => job
            .source
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned()),
    };
    if name.is_empty() {
        "transcript".into()
    } else {
        name
    }
}

/// Never overwrites: a second "meeting" becomes "meeting (2).txt".
fn write_outputs(
    dir: &Path,
    stem: &str,
    segments: &[Segment],
) -> std::io::Result<(PathBuf, PathBuf)> {
    let (txt, srt) = candidates(dir, stem, &["txt", "srt"])
        .find(|paths| paths.iter().all(|p| !p.exists()))
        .map(|mut paths| (paths.remove(0), paths.remove(0)))
        .expect("candidates never run out");
    save_atomic(&txt, output::to_txt(segments).as_bytes())?;
    save_atomic(&srt, output::to_srt(segments).as_bytes())?;
    Ok((txt, srt))
}

/// Sample index where each silence of at least 0.5 s below −35 dB ends.
fn silence_ends(samples: &[f32]) -> Vec<usize> {
    const FRAME: usize = SAMPLE_RATE / 50;
    const MIN_FRAMES: usize = 25;
    let threshold = 10f32.powf(-35.0 / 20.0);
    let mut ends = Vec::new();
    let mut quiet = 0;
    for (i, frame) in samples.chunks(FRAME).enumerate() {
        let rms = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt();
        if rms < threshold {
            quiet += 1;
        } else {
            if quiet >= MIN_FRAMES {
                ends.push(i * FRAME);
            }
            quiet = 0;
        }
    }
    ends
}

/// About 15-minute pieces, cut at the silence nearest each target when there is one.
fn plan_chunks(silences: &[usize], len: usize) -> Vec<(usize, usize)> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while len - start > MAX_LAST_CHUNK_S * SAMPLE_RATE {
        let target = start + CHUNK_S * SAMPLE_RATE;
        let cut = silences
            .iter()
            .copied()
            .filter(|&s| {
                s.abs_diff(target) <= SEARCH_S * SAMPLE_RATE
                    && s > start + MIN_CHUNK_S * SAMPLE_RATE
            })
            .min_by_key(|&s| s.abs_diff(target))
            .unwrap_or(target);
        chunks.push((start, cut));
        start = cut;
    }
    chunks.push((start, len));
    chunks
}

#[cfg(test)]
mod tests;
