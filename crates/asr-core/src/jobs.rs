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
    /// Local transcription model pinned to this job so resuming never mixes chunks across models.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional user group / subfolder name under the output directory.
    #[serde(default)]
    pub group: Option<String>,
    pub state: JobState,
}

/// What a watcher sees after every change.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Queue<'a> {
    pub jobs: &'a [Job],
    pub groups: &'a [String],
    pub paused: bool,
    /// Work is waiting or running and the queue isn't paused: keep the machine awake.
    pub busy: bool,
}

impl<'a> Queue<'a> {
    fn of(state: &'a State) -> Self {
        Queue {
            jobs: &state.jobs,
            groups: &state.groups,
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
    #[serde(default)]
    model: Option<String>,
    chunks: Vec<(usize, usize)>,
    done: Vec<Vec<Segment>>,
}

struct State {
    jobs: Vec<Job>,
    groups: Vec<String>,
    paused: bool,
    next_id: u64,
    default_model: String,
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

    /// Saves the job & group lists and tells the watcher.
    fn publish(&self, state: &State) {
        let saved = save_atomic(
            &self.paths.data.join("jobs.json"),
            &serde_json::to_vec(&state.jobs).unwrap_or_default(),
        );
        if let Err(e) = saved {
            eprintln!("không lưu được danh sách việc: {e}");
        }
        let saved_groups = save_atomic(
            &self.paths.data.join("groups.json"),
            &serde_json::to_vec(&state.groups).unwrap_or_default(),
        );
        if let Err(e) = saved_groups {
            eprintln!("không lưu được danh sách nhóm: {e}");
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
    /// closed are queued again. `load` runs when a job needs an engine (identified by model ID)
    /// and a different model or none is loaded: the engine is dropped whenever the queue runs
    /// empty, and a failed load is retried on the next job. `on_change` runs with the queue
    /// locked, so it must be quick and must not call back into this runner.
    pub fn start<L, C>(
        paths: Paths,
        default_model: String,
        load: L,
        on_change: C,
    ) -> std::io::Result<Self>
    where
        L: Fn(&str) -> Result<Box<dyn Transcriber>, TranscribeError> + Send + 'static,
        C: Fn(&Queue) + Send + Sync + 'static,
    {
        fs::create_dir_all(&paths.data)?;
        fs::create_dir_all(&paths.output)?;
        let mut jobs: Vec<Job> = fs::read(paths.data.join("jobs.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut groups: Vec<String> = fs::read(paths.data.join("groups.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        for job in &mut jobs {
            if matches!(job.state, JobState::Running { .. }) {
                job.state = JobState::Queued;
            }
            if let Some(ref g) = job.group
                && !groups.contains(g)
            {
                groups.push(g.clone());
            }
        }
        let next_id = jobs.iter().map(|j| j.id + 1).max().unwrap_or(1);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                jobs,
                groups,
                paused: false,
                next_id,
                default_model,
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

    pub fn default_model(&self) -> String {
        self.shared.lock().default_model.clone()
    }

    /// Sets the model used for newly enqueued jobs. Existing jobs keep their own pinned model.
    pub fn set_default_model(&self, model_id: String) {
        self.shared.lock().default_model = model_id;
    }

    pub fn jobs(&self) -> Vec<Job> {
        self.shared.lock().jobs.clone()
    }

    pub fn groups(&self) -> Vec<String> {
        self.shared.lock().groups.clone()
    }

    /// Creates a named group in the sidebar and its matching folder inside `paths.output`.
    pub fn create_group(&self, name: &str) -> Result<String, String> {
        let clean = sanitize_group_name(name)?;
        let dir = self.shared.paths.output.join(&clean);
        fs::create_dir_all(&dir).map_err(|e| format!("không tạo được thư mục nhóm: {e}"))?;
        let mut state = self.shared.lock();
        if !state.groups.contains(&clean) {
            state.groups.push(clean.clone());
            self.shared.publish(&state);
        }
        Ok(clean)
    }

    /// Returns the output folder path for a group, creating it if needed.
    pub fn group_dir(&self, name: &str) -> Result<PathBuf, String> {
        let clean = sanitize_group_name(name)?;
        let dir = self.shared.paths.output.join(&clean);
        fs::create_dir_all(&dir).map_err(|e| format!("không mở được thư mục nhóm: {e}"))?;
        Ok(dir)
    }

    /// Removes a group, moving any finished transcripts back to the root output folder.
    /// Non-empty folders are never force-deleted.
    pub fn delete_group(&self, name: &str) -> Result<(), String> {
        let mut state = self.shared.lock();
        state.groups.retain(|g| g != name);
        let root_out = self.shared.paths.output.clone();
        for job in &mut state.jobs {
            if job.group.as_deref() == Some(name) {
                if let JobState::Done { txt, srt } = &job.state
                    && txt.parent() != Some(root_out.as_path())
                    && let Ok((new_txt, new_srt)) = move_done_outputs(txt, srt, &root_out)
                {
                    job.state = JobState::Done {
                        txt: new_txt,
                        srt: new_srt,
                    };
                }
                job.group = None;
            }
        }
        fs::remove_dir(root_out.join(name)).ok();
        self.shared.publish(&state);
        Ok(())
    }

    /// Assigns (or unassigns) a job's group, creating the group folder and moving finished
    /// output files into that folder so disk layout stays in sync with the sidebar.
    pub fn set_job_group(&self, id: u64, group: Option<String>) -> Result<(), String> {
        let clean = match group.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(raw) => Some(sanitize_group_name(raw)?),
            None => None,
        };
        let target_dir = match &clean {
            Some(g) => self.shared.paths.output.join(g),
            None => self.shared.paths.output.clone(),
        };
        fs::create_dir_all(&target_dir).map_err(|e| format!("không tạo được thư mục nhóm: {e}"))?;

        let mut state = self.shared.lock();
        if let Some(ref g) = clean
            && !state.groups.contains(g)
        {
            state.groups.push(g.clone());
        }
        let Some(job) = state.jobs.iter_mut().find(|j| j.id == id) else {
            return Err("không tìm thấy bản ghi này".into());
        };
        if let JobState::Done { txt, srt } = &job.state
            && txt.parent() != Some(target_dir.as_path())
        {
            let (new_txt, new_srt) = move_done_outputs(txt, srt, &target_dir)?;
            job.state = JobState::Done {
                txt: new_txt,
                srt: new_srt,
            };
        }
        job.group = clean;
        self.shared.publish(&state);
        Ok(())
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
        self.enqueue_in_group(source, title, None)
    }

    pub fn enqueue_in_group(
        &self,
        source: PathBuf,
        title: Option<String>,
        group: Option<String>,
    ) -> Result<u64, Rejected> {
        if !audio::is_supported(&source) {
            return Err(Rejected::Unsupported);
        }
        let clean_group = group
            .as_deref()
            .and_then(|g| sanitize_group_name(g).ok());
        if let Some(ref g) = clean_group {
            fs::create_dir_all(self.shared.paths.output.join(g)).ok();
        }
        let mut state = self.shared.lock();
        if let Some(job) = state
            .jobs
            .iter()
            .find(|j| j.source == source && j.state.is_active())
        {
            return Err(Rejected::AlreadyQueued(job.id));
        }
        if let Some(ref g) = clean_group
            && !state.groups.contains(g)
        {
            state.groups.push(g.clone());
        }
        let id = state.next_id;
        state.next_id += 1;
        let model = Some(state.default_model.clone());
        state.jobs.push(Job {
            id,
            source,
            title,
            model,
            group: clean_group,
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

    /// Enqueues a fresh job for the same recording using the current default model,
    /// keeping the previous job and its transcript files intact for comparison.
    pub fn rerun(&self, id: u64) -> Result<u64, Rejected> {
        let (source, title, group) = {
            let state = self.shared.lock();
            let Some(job) = state.jobs.iter().find(|j| j.id == id) else {
                return Err(Rejected::Unsupported);
            };
            (job.source.clone(), job.title.clone(), job.group.clone())
        };
        self.enqueue_in_group(source, title, group)
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

    /// Removes every non-running job in a single lock + publish cycle and returns their IDs.
    pub fn clear_removable(&self) -> Vec<u64> {
        let mut state = self.shared.lock();
        let mut removed = Vec::new();
        state.jobs.retain(|j| {
            if matches!(j.state, JobState::Running { .. }) {
                true
            } else {
                removed.push(j.id);
                false
            }
        });
        if !removed.is_empty() {
            for &id in &removed {
                fs::remove_dir_all(self.shared.work_dir(id)).ok();
            }
            self.shared.publish(&state);
        }
        removed
    }
}

fn run_worker<L>(shared: &Arc<Shared>, load: L)
where
    L: Fn(&str) -> Result<Box<dyn Transcriber>, TranscribeError>,
{
    let mut loaded: Option<(String, Box<dyn Transcriber>)> = None;
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
                    None => loaded = None,
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

        let job_model = job
            .model
            .as_deref()
            .unwrap_or(crate::models::DEFAULT_MODEL_ID);
        if loaded.as_ref().is_some_and(|(id, _)| id != job_model) {
            loaded = None;
        }
        let outcome = match &loaded {
            Some(_) => Ok(()),
            None => load(job_model)
                .map(|e| loaded = Some((job_model.to_owned(), e)))
                .map_err(|e| e.to_string()),
        }
        .and_then(|()| {
            process(
                shared,
                loaded.as_ref().expect("loaded above").1.as_ref(),
                &job,
                job_model,
            )
        });
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

fn process(
    shared: &Arc<Shared>,
    engine: &dyn Transcriber,
    job: &Job,
    job_model: &str,
) -> Result<JobState, String> {
    let fingerprint = fingerprint(&job.source)?;
    let work = shared.work_dir(job.id);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let checkpoint_path = work.join("checkpoint.json");
    let pcm_path = work.join("audio_16k.pcm");

    let saved_checkpoint = fs::read(&checkpoint_path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Checkpoint>(&b).ok())
        .filter(|c| {
            c.fingerprint == fingerprint
                && c.model.as_deref().unwrap_or(crate::models::DEFAULT_MODEL_ID) == job_model
        });

    // Reuse cached 16 kHz PCM16 on resume so pausing/restarting skips re-decoding long files.
    let pcm = match (saved_checkpoint.as_ref(), audio::Pcm16File::open_existing(&pcm_path)) {
        (Some(cp), Some(existing))
            if cp.chunks.last().map(|&(_, end)| end) == Some(existing.len()) =>
        {
            existing
        }
        _ => audio::decode_to_pcm16(&job.source, &pcm_path)?,
    };

    let mut checkpoint = match saved_checkpoint {
        Some(cp) if cp.chunks.last().map(|&(_, end)| end) == Some(pcm.len()) => cp,
        _ => Checkpoint {
            fingerprint,
            model: Some(job_model.to_owned()),
            chunks: plan_chunks(&pcm.silence_ends()?, pcm.len()),
            done: Vec::new(),
        },
    };

    let total = pcm.len().max(1);
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
        let chunk_samples = pcm.read_range(start, end)?;
        let control = Control {
            cancel: shared.interrupt.clone(),
            on_progress: {
                let progress_of = progress_of.clone();
                let report = report_progress(shared.clone(), job.id, total, resumed, started);
                Arc::new(move |p| report(progress_of(index, p)))
            },
        };
        match engine.transcribe(&chunk_samples, &control) {
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
    let out_dir = match &job.group {
        Some(g) => {
            let dir = shared.paths.output.join(g);
            fs::create_dir_all(&dir).map_err(|e| format!("không tạo được thư mục nhóm: {e}"))?;
            dir
        }
        None => shared.paths.output.clone(),
    };
    let (txt, srt) = write_outputs(&out_dir, &output_name(job), &segments)
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

/// Sanitizes a user group name so it is safe as a single folder name under `paths.output`.
fn sanitize_group_name(raw: &str) -> Result<String, String> {
    let clean: String = raw
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
        .to_owned();
    if clean.is_empty() {
        Err("tên nhóm không hợp lệ".into())
    } else {
        Ok(clean)
    }
}

/// Moves a finished job's transcript and any sibling notes/exports into `target_dir`.
fn move_done_outputs(txt: &Path, srt: &Path, target_dir: &Path) -> Result<(PathBuf, PathBuf), String> {
    let old_stem = txt
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "transcript".into());
    let (new_txt, new_srt) = candidates(target_dir, &old_stem, &["txt", "srt"])
        .find(|paths| paths.iter().all(|p| !p.exists()))
        .map(|mut paths| (paths.remove(0), paths.remove(0)))
        .expect("candidates never run out");
    let new_stem = new_txt
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or(old_stem.clone());

    if txt.exists() {
        fs::rename(txt, &new_txt).map_err(|e| format!("không chuyển được file txt: {e}"))?;
    }
    if srt.exists() {
        fs::rename(srt, &new_srt).map_err(|e| format!("không chuyển được file srt: {e}"))?;
    }
    if let Some(old_dir) = txt.parent() {
        for suffix in ["note.json", "note.md", "xlsx", "docx"] {
            let old_sibling = old_dir.join(format!("{old_stem}.{suffix}"));
            if old_sibling.exists() {
                let new_sibling = target_dir.join(format!("{new_stem}.{suffix}"));
                let _ = fs::rename(&old_sibling, &new_sibling);
            }
        }
    }
    Ok((new_txt, new_srt))
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
#[cfg(test)]
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
