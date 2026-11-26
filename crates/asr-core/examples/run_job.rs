//! Real engine through the real queue: run_job <models dir> <work dir> <audio file>
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use asr_core::models::{DEFAULT_MODEL_ID, ModelStore};
use asr_core::transcriber::ResourceProfile;
use asr_core::{JobRunner, JobState, Paths, Queue};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let store = ModelStore::new(PathBuf::from(&a[1]));
    let work = Path::new(&a[2]);
    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let runner = JobRunner::start(
        Paths {
            data: work.join("data"),
            output: work.join("out"),
        },
        DEFAULT_MODEL_ID.to_owned(),
        move |id| store.load(id, ResourceProfile::default(), None),
        move |queue: &Queue| {
            tx.lock()
                .unwrap()
                .send(queue.jobs.last().unwrap().state.clone())
                .ok();
        },
    )
    .unwrap();
    let t = std::time::Instant::now();
    runner.enqueue(PathBuf::from(&a[3])).unwrap();
    for state in rx {
        match state {
            JobState::Running { percent, eta_s } => eprint!("\r{percent}% eta {eta_s:?}s   "),
            JobState::Queued => {}
            other => {
                println!("\n{other:?} after {:.0}s", t.elapsed().as_secs_f64());
                // Stay alive so the memory held after a job can be measured from outside.
                std::thread::sleep(std::time::Duration::from_secs(
                    std::env::var("HOLD_S").map_or(0, |v| v.parse().unwrap()),
                ));
                return;
            }
        }
    }
}
