use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;
use std::time::Duration;

use super::*;

const WAIT: Duration = Duration::from_secs(20);

/// Answers "chunk at <first sample value>", and can block or fail on demand.
#[derive(Clone, Default)]
struct Fake {
    calls: Arc<Mutex<Vec<usize>>>,
    /// While set, transcribe spins until interrupted, like a long-running chunk.
    block: Arc<AtomicBool>,
    fail: Arc<AtomicBool>,
}

impl Transcriber for Fake {
    fn transcribe(
        &self,
        samples: &[f32],
        control: &Control,
    ) -> Result<Vec<Segment>, TranscribeError> {
        self.calls.lock().unwrap().push(samples.len());
        while self.block.load(Ordering::Relaxed) {
            if control.cancel.load(Ordering::Relaxed) {
                return Err(TranscribeError::Cancelled);
            }
            thread::sleep(Duration::from_millis(2));
        }
        if self.fail.load(Ordering::Relaxed) {
            return Err(TranscribeError::Engine("hỏng".into()));
        }
        (control.on_progress)(50);
        Ok(vec![Segment {
            start_ms: 0,
            end_ms: 1000,
            text: "xin chào".into(),
        }])
    }
}

fn write_wav(path: &Path, seconds: u32) -> PathBuf {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for _ in 0..seconds * 16_000 {
        w.write_sample(1000i16).unwrap();
    }
    w.finalize().unwrap();
    path.to_owned()
}

struct Rig {
    runner: JobRunner,
    fake: Fake,
    dir: tempfile::TempDir,
    changes: mpsc::Receiver<Vec<Job>>,
}

impl Rig {
    fn new() -> Self {
        Self::in_dir(tempfile::tempdir().unwrap(), Fake::default())
    }

    fn in_dir(dir: tempfile::TempDir, fake: Fake) -> Self {
        let (tx, changes) = mpsc::channel();
        let tx = Mutex::new(tx);
        let engine = fake.clone();
        let paths = Paths {
            data: dir.path().join("data"),
            output: dir.path().join("out"),
        };
        let runner = JobRunner::start(
            paths,
            crate::models::DEFAULT_MODEL_ID.to_owned(),
            move |_model_id| Ok(Box::new(engine.clone()) as Box<dyn Transcriber>),
            move |queue: &Queue| {
                tx.lock().unwrap().send(queue.jobs.to_vec()).ok();
            },
        )
        .unwrap();
        Self {
            runner,
            fake,
            dir,
            changes,
        }
    }

    fn wav(&self, name: &str, seconds: u32) -> PathBuf {
        write_wav(&self.dir.path().join(name), seconds)
    }

    fn wait_for(&self, what: &str, done: impl Fn(&[Job]) -> bool) -> Vec<Job> {
        let deadline = Instant::now() + WAIT;
        loop {
            let jobs = self.runner.jobs();
            if done(&jobs) {
                return jobs;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "timed out waiting for {what}: {jobs:?}");
            self.changes
                .recv_timeout(left.min(Duration::from_millis(50)))
                .ok();
        }
    }

    fn wait_state(&self, id: u64, what: &str, want: impl Fn(&JobState) -> bool) -> JobState {
        let jobs = self.wait_for(what, |jobs| {
            jobs.iter().any(|j| j.id == id && want(&j.state))
        });
        jobs.into_iter().find(|j| j.id == id).unwrap().state
    }
}

fn is_done(s: &JobState) -> bool {
    matches!(s, JobState::Done { .. })
}

fn is_running(s: &JobState) -> bool {
    matches!(s, JobState::Running { .. })
}

#[test]
fn a_job_runs_to_finished_transcripts_with_both_formats() {
    let rig = Rig::new();
    let id = rig.runner.enqueue(rig.wav("họp tổ.wav", 2)).unwrap();
    let JobState::Done { txt, srt } = rig.wait_state(id, "done", is_done) else {
        unreachable!()
    };
    assert!(txt.ends_with("họp tổ.txt"));
    assert_eq!(
        fs::read_to_string(txt).unwrap(),
        "[00:00:00 --> 00:00:01] xin chào\n"
    );
    assert!(
        fs::read_to_string(srt)
            .unwrap()
            .contains("00:00:00,000 --> 00:00:01,000")
    );
}

#[test]
fn the_same_name_twice_never_overwrites() {
    let rig = Rig::new();
    let source = rig.wav("a.wav", 1);
    let first = rig.runner.enqueue(source.clone()).unwrap();
    rig.wait_state(first, "first", is_done);
    let second = rig.runner.enqueue(source).unwrap();
    let JobState::Done { txt, .. } = rig.wait_state(second, "second", is_done) else {
        unreachable!()
    };
    assert!(txt.ends_with("a (2).txt"));
}

#[test]
fn a_title_names_the_transcript_and_cannot_escape_the_folder() {
    let rig = Rig::new();
    let id = rig.runner.enqueue_titled(
        rig.wav("20261109 141000-AB12.m4a.wav", 1),
        Some("Meeting kế toán 09/11/2026".into()),
    ).unwrap();
    let JobState::Done { txt, .. } = rig.wait_state(id, "done", is_done) else {
        unreachable!()
    };
    assert_eq!(
        txt,
        rig.dir.path().join("out/Meeting kế toán 09-11-2026.txt")
    );

    let id = rig
        .runner
        .enqueue_titled(rig.wav("x.wav", 1), Some("../../.ẩn".into())).unwrap();
    let JobState::Done { txt, .. } = rig.wait_state(id, "done", is_done) else {
        unreachable!()
    };
    assert_eq!(txt.parent().unwrap(), rig.dir.path().join("out"));
}

/// Counts how many engines exist, to see when the worker lets go of one.
struct Counted {
    live: Arc<AtomicUsize>,
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Transcriber for Counted {
    fn transcribe(&self, _: &[f32], _: &Control) -> Result<Vec<Segment>, TranscribeError> {
        Ok(Vec::new())
    }
}

#[test]
fn the_engine_is_released_when_the_queue_empties_and_loaded_again_for_new_work() {
    let dir = tempfile::tempdir().unwrap();
    let source = write_wav(&dir.path().join("a.wav"), 1);
    let live = Arc::new(AtomicUsize::new(0));
    let loads = Arc::new(AtomicUsize::new(0));
    let (live_in, loads_in) = (live.clone(), loads.clone());
    let runner = JobRunner::start(
        Paths {
            data: dir.path().join("data"),
            output: dir.path().join("out"),
        },
        crate::models::DEFAULT_MODEL_ID.to_owned(),
        move |_model_id| {
            live_in.fetch_add(1, Ordering::SeqCst);
            loads_in.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Counted {
                live: live_in.clone(),
            }) as Box<dyn Transcriber>)
        },
        |_: &Queue| {},
    )
    .unwrap();
    let settled = |want_jobs: usize| {
        let deadline = Instant::now() + WAIT;
        while runner.jobs().iter().filter(|j| is_done(&j.state)).count() < want_jobs
            || live.load(Ordering::SeqCst) != 0
        {
            assert!(
                Instant::now() < deadline,
                "engine still held: {:?}",
                runner.jobs()
            );
            thread::sleep(Duration::from_millis(2));
        }
    };
    runner.enqueue(source.clone()).unwrap();
    settled(1);
    assert_eq!(loads.load(Ordering::SeqCst), 1);
    runner.enqueue(source).unwrap();
    settled(2);
    assert_eq!(loads.load(Ordering::SeqCst), 2);
}

#[test]
fn several_queued_jobs_share_one_loaded_engine() {
    let dir = tempfile::tempdir().unwrap();
    let source = write_wav(&dir.path().join("a.wav"), 1);
    let live = Arc::new(AtomicUsize::new(0));
    let loads = Arc::new(AtomicUsize::new(0));
    let (live_in, loads_in) = (live, loads.clone());
    let runner = JobRunner::start(
        Paths {
            data: dir.path().join("data"),
            output: dir.path().join("out"),
        },
        crate::models::DEFAULT_MODEL_ID.to_owned(),
        move |_model_id| {
            live_in.fetch_add(1, Ordering::SeqCst);
            loads_in.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Counted {
                live: live_in.clone(),
            }) as Box<dyn Transcriber>)
        },
        |_: &Queue| {},
    )
    .unwrap();
    runner.set_paused(true);
    for name in ["b.wav", "c.wav", "d.wav"] {
        std::fs::copy(&source, dir.path().join(name)).unwrap();
        runner.enqueue(dir.path().join(name)).unwrap();
    }
    runner.set_paused(false);
    let deadline = Instant::now() + WAIT;
    while runner.jobs().iter().filter(|j| is_done(&j.state)).count() < 3 {
        assert!(Instant::now() < deadline, "{:?}", runner.jobs());
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(loads.load(Ordering::SeqCst), 1);
}

#[test]
fn a_failed_job_does_not_block_the_next_one() {
    let rig = Rig::new();
    let missing = rig.runner.enqueue(rig.dir.path().join("không có.wav")).unwrap();
    let good = rig.runner.enqueue(rig.wav("b.wav", 1)).unwrap();
    rig.wait_state(missing, "failure", |s| matches!(s, JobState::Failed { .. }));
    rig.wait_state(good, "next job", is_done);
}

#[test]
fn an_engine_error_marks_the_job_failed_and_retry_works() {
    let rig = Rig::new();
    rig.fake.fail.store(true, Ordering::Relaxed);
    let id = rig.runner.enqueue(rig.wav("c.wav", 1)).unwrap();
    let state = rig.wait_state(id, "failure", |s| matches!(s, JobState::Failed { .. }));
    assert!(matches!(state, JobState::Failed { message } if message.contains("hỏng")));
    rig.fake.fail.store(false, Ordering::Relaxed);
    rig.runner.retry(id);
    rig.wait_state(id, "retry", is_done);
}

#[test]
fn cancel_stops_the_running_job_and_the_queue_moves_on() {
    let rig = Rig::new();
    rig.fake.block.store(true, Ordering::Relaxed);
    let running = rig.runner.enqueue(rig.wav("d.wav", 1)).unwrap();
    let next = rig.runner.enqueue(rig.wav("e.wav", 1)).unwrap();
    rig.wait_state(running, "start", is_running);
    rig.runner.cancel(running);
    rig.wait_for("worker to leave the cancelled job", |jobs| {
        jobs.iter()
            .any(|j| j.id == next && j.state != JobState::Queued)
    });
    rig.fake.block.store(false, Ordering::Relaxed);
    rig.wait_state(next, "next", is_done);
    assert_eq!(rig.runner.jobs()[0].state, JobState::Cancelled);
}

#[test]
fn pause_interrupts_the_chunk_and_resume_redoes_it() {
    let rig = Rig::new();
    rig.fake.block.store(true, Ordering::Relaxed);
    let id = rig.runner.enqueue(rig.wav("f.wav", 1)).unwrap();
    rig.wait_state(id, "start", is_running);
    // Running starts before decoding; pause only once the chunk is actually in progress.
    let deadline = Instant::now() + WAIT;
    while rig.fake.calls.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "chunk never started");
        thread::sleep(Duration::from_millis(2));
    }
    rig.runner.set_paused(true);
    // The interrupted attempt returns on its own: the block is still set here.
    thread::sleep(Duration::from_millis(30));
    rig.fake.block.store(false, Ordering::Relaxed);
    thread::sleep(Duration::from_millis(50));
    assert!(
        is_running(&rig.runner.jobs()[0].state),
        "must not finish while paused"
    );
    rig.runner.set_paused(false);
    rig.wait_state(id, "resume", is_done);
    assert_eq!(rig.fake.calls.lock().unwrap().len(), 2);
}

#[test]
fn nothing_starts_while_paused() {
    let rig = Rig::new();
    rig.runner.set_paused(true);
    let id = rig.runner.enqueue(rig.wav("g.wav", 1)).unwrap();
    thread::sleep(Duration::from_millis(50));
    assert_eq!(rig.runner.jobs()[0].state, JobState::Queued);
    rig.runner.set_paused(false);
    rig.wait_state(id, "resume", is_done);
}

#[test]
fn restarting_requeues_a_job_left_running() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    fs::create_dir_all(&data).unwrap();
    let source = dir.path().join("h.wav");
    write_wav(&source, 1);
    let job = Job {
        id: 7,
        source: source.clone(),
        title: None,
        model: None,
        group: None,
        state: JobState::Running {
            percent: 40,
            eta_s: Some(9),
        },
    };
    fs::write(data.join("jobs.json"), serde_json::to_vec(&[job]).unwrap()).unwrap();
    let rig = Rig::in_dir(dir, Fake::default());
    rig.wait_state(7, "pickup", is_done);
    assert_eq!(
        rig.runner.enqueue(source).unwrap(),
        8,
        "ids continue after the saved jobs"
    );
}

#[test]
fn a_checkpoint_skips_finished_chunks_and_keeps_their_timing() {
    let rig = Rig::new();
    let source = rig.wav("i.wav", 2);
    rig.runner.set_paused(true);
    let id = rig.runner.enqueue(source.clone()).unwrap();
    let work = rig.dir.path().join("data/work").join(id.to_string());
    fs::create_dir_all(&work).unwrap();
    let checkpoint = Checkpoint {
        fingerprint: fingerprint(&source).unwrap(),
        model: Some(crate::models::DEFAULT_MODEL_ID.to_owned()),
        chunks: vec![(0, 16_000), (16_000, 32_000)],
        done: vec![vec![Segment {
            start_ms: 0,
            end_ms: 900,
            text: "đã xong".into(),
        }]],
    };
    fs::write(
        work.join("checkpoint.json"),
        serde_json::to_vec(&checkpoint).unwrap(),
    )
    .unwrap();
    rig.runner.set_paused(false);
    let JobState::Done { txt, .. } = rig.wait_state(id, "done", is_done) else {
        unreachable!()
    };
    assert_eq!(
        *rig.fake.calls.lock().unwrap(),
        [16_000],
        "only the second chunk runs"
    );
    assert_eq!(
        fs::read_to_string(txt).unwrap(),
        "[00:00:00 --> 00:00:00] đã xong\n[00:00:01 --> 00:00:02] xin chào\n"
    );
    assert!(!work.exists(), "checkpoint is removed once finished");
}

#[test]
fn a_checkpoint_for_a_changed_source_is_ignored() {
    let rig = Rig::new();
    let source = rig.wav("j.wav", 2);
    rig.runner.set_paused(true);
    let id = rig.runner.enqueue(source).unwrap();
    let work = rig.dir.path().join("data/work").join(id.to_string());
    fs::create_dir_all(&work).unwrap();
    let stale = Checkpoint {
        fingerprint: "khác".into(),
        model: None,
        chunks: vec![(0, 1)],
        done: vec![vec![]],
    };
    fs::write(
        work.join("checkpoint.json"),
        serde_json::to_vec(&stale).unwrap(),
    )
    .unwrap();
    rig.runner.set_paused(false);
    rig.wait_state(id, "done", is_done);
    assert_eq!(*rig.fake.calls.lock().unwrap(), [32_000]);
}

#[test]
fn short_audio_stays_in_one_chunk() {
    let len = MAX_LAST_CHUNK_S * SAMPLE_RATE;
    assert_eq!(plan_chunks(&[], len), [(0, len)]);
}

#[test]
fn long_audio_splits_at_the_nearest_silence_or_hard_cuts() {
    let sr = SAMPLE_RATE;
    let chunks = plan_chunks(&[290 * sr, 330 * sr], 500 * sr);
    assert_eq!(chunks, [(0, 290 * sr), (290 * sr, 500 * sr)]);
    let chunks = plan_chunks(&[], 700 * sr);
    assert_eq!(chunks, [(0, CHUNK_S * sr), (CHUNK_S * sr, 700 * sr)]);
}

#[test]
fn silence_ends_finds_a_long_gap_but_not_a_short_one() {
    let loud = |n: usize| vec![0.5f32; n];
    let quiet = |n: usize| vec![0.0f32; n];
    let sr = SAMPLE_RATE;
    let samples = [loud(sr), quiet(sr / 10), loud(sr), quiet(sr), loud(sr)].concat();
    let ends = silence_ends(&samples);
    assert_eq!(ends.len(), 1);
    assert!(ends[0].abs_diff(sr * 3 + sr / 10) < sr / 25, "{ends:?}");
}

#[test]
fn a_file_already_waiting_is_refused_but_a_finished_one_may_run_again() {
    let rig = Rig::new();
    rig.runner.set_paused(true);
    let source = rig.wav("i.wav", 1);
    let id = rig.runner.enqueue(source.clone()).unwrap();
    assert_eq!(
        rig.runner.enqueue(source.clone()),
        Err(Rejected::AlreadyQueued(id))
    );
    rig.runner.set_paused(false);
    rig.wait_state(id, "done", is_done);
    assert!(rig.runner.enqueue(source).is_ok());
}

#[test]
fn a_file_the_decoder_is_not_meant_for_is_refused() {
    let rig = Rig::new();
    let doc = rig.dir.path().join("biên bản.docx");
    fs::write(&doc, "x").unwrap();
    assert_eq!(rig.runner.enqueue(doc), Err(Rejected::Unsupported));
    assert!(rig.runner.jobs().is_empty());
}

#[test]
fn busy_means_work_waiting_and_not_paused() {
    let rig = Rig::new();
    assert!(!rig.runner.is_busy());
    rig.fake.block.store(true, Ordering::Relaxed);
    let id = rig.runner.enqueue(rig.wav("j.wav", 1)).unwrap();
    rig.wait_state(id, "running", is_running);
    assert!(rig.runner.is_busy());
    rig.runner.set_paused(true);
    assert!(!rig.runner.is_busy());
    rig.runner.set_paused(false);
    rig.fake.block.store(false, Ordering::Relaxed);
    rig.wait_state(id, "done", is_done);
    assert!(!rig.runner.is_busy());
}

#[test]
fn cancelling_a_queued_job_never_runs_it() {
    let rig = Rig::new();
    rig.runner.set_paused(true);
    let id = rig.runner.enqueue(rig.wav("k.wav", 1)).unwrap();
    rig.runner.cancel(id);
    rig.runner.set_paused(false);
    assert_eq!(rig.runner.jobs()[0].state, JobState::Cancelled);
    assert!(rig.fake.calls.lock().unwrap().is_empty());
}

#[test]
fn a_finished_job_hands_out_its_transcript_and_a_removed_one_does_not() {
    let rig = Rig::new();
    let id = rig.runner.enqueue(rig.wav("l.wav", 1)).unwrap();
    assert!(rig.runner.transcript(id).is_err(), "not finished yet");
    rig.wait_state(id, "done", is_done);
    let t = rig.runner.transcript(id).unwrap();
    assert_eq!(t.lines().unwrap()[0].text, "xin chào");
    assert_eq!(t.source(), rig.dir.path().join("l.wav"));
    rig.runner.remove(id);
    assert!(rig.runner.jobs().is_empty());
    assert!(rig.runner.transcript(id).is_err());
}

#[test]
fn resuming_a_checkpointed_job_keeps_its_original_model_and_new_jobs_use_the_new_default() {
    let dir = tempfile::tempdir().unwrap();
    let loaded_models = Arc::new(Mutex::new(Vec::<String>::new()));
    let loaded_in = loaded_models.clone();
    let fake = Fake::default();
    let fake_in = fake.clone();
    let runner = JobRunner::start(
        Paths {
            data: dir.path().join("data"),
            output: dir.path().join("out"),
        },
        "whisper-large-v3-turbo".to_owned(),
        move |model_id| {
            loaded_in.lock().unwrap().push(model_id.to_owned());
            Ok(Box::new(fake_in.clone()) as Box<dyn Transcriber>)
        },
        |_: &Queue| {},
    )
    .unwrap();

    let first_src = write_wav(&dir.path().join("first.wav"), 2);
    let second_src = write_wav(&dir.path().join("second.wav"), 1);
    runner.set_paused(true);
    let first_id = runner.enqueue(first_src.clone()).unwrap();
    // Simulate a checkpoint written by Turbo before the user switches default model.
    let work = dir.path().join("data/work").join(first_id.to_string());
    fs::create_dir_all(&work).unwrap();
    let checkpoint = Checkpoint {
        fingerprint: fingerprint(&first_src).unwrap(),
        model: Some("whisper-large-v3-turbo".to_owned()),
        chunks: vec![(0, 16_000), (16_000, 32_000)],
        done: vec![vec![Segment {
            start_ms: 0,
            end_ms: 900,
            text: "khúc turbo".into(),
        }]],
    };
    fs::write(
        work.join("checkpoint.json"),
        serde_json::to_vec(&checkpoint).unwrap(),
    )
    .unwrap();

    // User switches default model to PhoWhisper and adds a new job.
    runner.set_default_model("phowhisper-medium-q5_0".to_owned());
    let second_id = runner.enqueue(second_src).unwrap();
    runner.set_paused(false);

    let deadline = Instant::now() + WAIT;
    while runner.jobs().iter().filter(|j| is_done(&j.state)).count() < 2 {
        assert!(Instant::now() < deadline, "{:?}", runner.jobs());
        thread::sleep(Duration::from_millis(2));
    }

    assert_eq!(
        *loaded_models.lock().unwrap(),
        vec!["whisper-large-v3-turbo", "phowhisper-medium-q5_0"]
    );
    // First job resumed its second chunk only (16_000 samples); second job ran full 16_000 samples.
    assert_eq!(*fake.calls.lock().unwrap(), [16_000, 16_000]);
    let first_text = runner.transcript(first_id).unwrap().text().unwrap();
    assert!(first_text.contains("khúc turbo"));
    assert_eq!(
        runner.jobs().into_iter().find(|j| j.id == second_id).unwrap().model.as_deref(),
        Some("phowhisper-medium-q5_0")
    );
}

#[test]
fn a_checkpoint_from_a_different_model_is_ignored() {
    let rig = Rig::new();
    let source = rig.wav("mismatch.wav", 2);
    rig.runner.set_paused(true);
    let id = rig.runner.enqueue(source.clone()).unwrap();
    let work = rig.dir.path().join("data/work").join(id.to_string());
    fs::create_dir_all(&work).unwrap();
    let wrong_model = Checkpoint {
        fingerprint: fingerprint(&source).unwrap(),
        model: Some("phowhisper-medium-q5_0".to_owned()),
        chunks: vec![(0, 16_000), (16_000, 32_000)],
        done: vec![vec![Segment {
            start_ms: 0,
            end_ms: 900,
            text: "khác model".into(),
        }]],
    };
    fs::write(
        work.join("checkpoint.json"),
        serde_json::to_vec(&wrong_model).unwrap(),
    )
    .unwrap();
    rig.runner.set_paused(false);
    rig.wait_state(id, "done", is_done);
    assert_eq!(*rig.fake.calls.lock().unwrap(), [32_000]);
}

#[test]
fn groups_create_folders_and_move_finished_outputs() {
    let rig = Rig::new();
    let group = rig.runner.create_group("Kế toán / Q4").unwrap();
    assert_eq!(group, "Kế toán - Q4");
    assert!(rig.dir.path().join("out").join(&group).is_dir());

    let source = rig.wav("hop.wav", 1);
    let id = rig.runner.enqueue(source).unwrap();
    rig.wait_state(id, "done", is_done);

    rig.runner.set_job_group(id, Some(group.clone())).unwrap();
    let job = rig.runner.jobs().into_iter().find(|j| j.id == id).unwrap();
    assert_eq!(job.group.as_deref(), Some(group.as_str()));
    let JobState::Done { txt, srt } = job.state else {
        panic!("expected done job");
    };
    assert_eq!(txt.parent(), Some(rig.dir.path().join("out").join(&group).as_path()));
    assert!(txt.exists());
    assert!(srt.exists());

    rig.runner.delete_group(&group).unwrap();
    let job_after = rig.runner.jobs().into_iter().find(|j| j.id == id).unwrap();
    assert_eq!(job_after.group, None);
    let JobState::Done { txt: txt2, .. } = job_after.state else {
        panic!("expected done job");
    };
    assert_eq!(txt2.parent(), Some(rig.dir.path().join("out").as_path()));
    assert!(txt2.exists());
}
