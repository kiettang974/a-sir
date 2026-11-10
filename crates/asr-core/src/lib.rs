//! Speech-to-text jobs. `JobRunner` is the module a UI talks to.

pub mod ai;
pub mod audio;
pub mod clean;
pub mod glossary;
mod jobs;
pub mod models;
pub mod output;
pub mod summary;
pub mod transcriber;
pub mod transcript;
pub mod voice_memos;
pub mod workspace;

pub use jobs::{Job, JobRunner, JobState, Paths, Queue, Rejected};
