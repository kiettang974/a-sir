#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Mutex;

use asr_core::ai::{AiSettings, Provider};
use asr_core::glossary::Term;
use asr_core::summary::Draft;
use asr_core::transcript::Line;
use asr_core::voice_memos::{self, ListError, Recording};
use asr_core::workspace::Workspace;
use asr_core::{Job, JobRunner, Paths, Queue, audio, models};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

struct App {
    runner: JobRunner,
    workspace: Workspace,
    models_dir: PathBuf,
    output_dir: PathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    jobs: Vec<Job>,
    paused: bool,
    busy: bool,
    models_ready: bool,
    output_dir: PathBuf,
    audio_extensions: &'static [&'static str],
}

#[tauri::command]
fn snapshot(app: State<App>) -> Snapshot {
    Snapshot {
        jobs: app.runner.jobs(),
        paused: app.runner.is_paused(),
        busy: app.runner.is_busy(),
        models_ready: models::are_ready(&app.models_dir),
        output_dir: app.output_dir.clone(),
        audio_extensions: &audio::EXTENSIONS,
    }
}

/// Adds what it can and says why the rest were skipped.
#[tauri::command]
fn enqueue(app: State<App>, paths: Vec<PathBuf>) -> Vec<String> {
    paths
        .into_iter()
        .filter_map(|path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            app.runner.enqueue(path).err().map(|e| format!("{name}: {e}"))
        })
        .collect()
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
enum MemoList {
    Ok { recordings: Vec<Recording> },
    NoPermission,
    Failed { message: String },
}

#[tauri::command]
fn list_voice_memos(handle: AppHandle) -> MemoList {
    let dir = match handle.path().home_dir() {
        Ok(home) => voice_memos::folder(&home),
        Err(e) => {
            return MemoList::Failed {
                message: e.to_string(),
            };
        }
    };
    match voice_memos::list(&dir) {
        Ok(recordings) => MemoList::Ok { recordings },
        Err(ListError::NoPermission) => MemoList::NoPermission,
        Err(ListError::Other(message)) => MemoList::Failed { message },
    }
}

#[derive(Deserialize)]
struct MemoChoice {
    path: PathBuf,
    title: String,
}

#[tauri::command]
fn enqueue_voice_memos(app: State<App>, choices: Vec<MemoChoice>) -> Vec<String> {
    choices
        .into_iter()
        .filter_map(|choice| {
            let title = choice.title.clone();
            app.runner
                .enqueue_titled(choice.path, Some(choice.title))
                .err()
                .map(|e| format!("{title}: {e}"))
        })
        .collect()
}

/// Only the person can grant this, in System Settings, and macOS wants the app reopened after.
#[tauri::command]
fn open_full_disk_settings() -> Result<(), String> {
    Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
        .status()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn relaunch(handle: AppHandle) {
    handle.restart();
}

#[tauri::command]
fn cancel(app: State<App>, id: u64) {
    app.runner.cancel(id);
}

#[tauri::command]
fn retry(app: State<App>, id: u64) {
    app.runner.retry(id);
}

#[tauri::command]
fn remove(app: State<App>, id: u64) {
    app.runner.remove(id);
}

#[tauri::command]
fn set_paused(app: State<App>, paused: bool) {
    app.runner.set_paused(paused);
}

#[derive(Serialize)]
struct TranscriptView {
    text: String,
    lines: Vec<Line>,
}

/// The transcript of a finished job. Takes an id, not a path, so the page can't read other files.
#[tauri::command]
fn read_transcript(app: State<App>, id: u64) -> Result<TranscriptView, String> {
    let text = app.runner.transcript(id)?.text()?;
    let lines = asr_core::transcript::lines(&text);
    Ok(TranscriptView { text, lines })
}

/// Writes the cleaned copy beside the transcript and returns its path.
#[tauri::command]
fn clean_transcript(app: State<App>, id: u64) -> Result<PathBuf, String> {
    app.runner.transcript(id)?.write_cleaned()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiView {
    #[serde(flatten)]
    settings: AiSettings,
    /// A masked hint for each provider that has a key. The key itself never reaches the page.
    keys: BTreeMap<Provider, String>,
}

#[tauri::command]
fn ai_settings(app: State<App>) -> Result<AiView, String> {
    Ok(AiView {
        settings: app.workspace.ai_settings(),
        keys: app.workspace.key_hints().map_err(|e| e.to_string())?,
    })
}

#[tauri::command]
fn save_ai_settings(app: State<App>, settings: AiSettings) -> Result<(), String> {
    app.workspace.save_ai_settings(&settings)
}

#[tauri::command]
fn set_api_key(app: State<App>, provider: Provider, key: String) -> Result<(), String> {
    app.workspace.set_key(provider, &key).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_api_key(app: State<App>, provider: Provider) -> Result<(), String> {
    app.workspace.delete_key(provider).map_err(|e| e.to_string())
}

/// Runs blocking network work off the main thread.
async fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn test_provider(
    app: State<'_, App>,
    provider: Provider,
    model: String,
) -> Result<(), String> {
    let client = app
        .workspace
        .client(provider, Some(&model))
        .map_err(|e| e.to_string())?;
    off_thread(move || client.ping().map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn list_models(app: State<'_, App>, provider: Provider) -> Result<Vec<String>, String> {
    let client = app
        .workspace
        .client(provider, Some(""))
        .map_err(|e| e.to_string())?;
    off_thread(move || client.list_models().map_err(|e| e.to_string())).await
}

#[tauri::command]
fn get_glossary(app: State<App>) -> Vec<Term> {
    app.workspace.glossary()
}

#[tauri::command]
fn save_glossary(app: State<App>, terms: Vec<Term>) -> Result<(), String> {
    app.workspace.save_glossary(&terms)
}

/// Adds the file's terms to the list on screen and returns the result.
#[tauri::command]
fn import_glossary(app: State<App>, terms: Vec<Term>, path: PathBuf) -> Result<Vec<Term>, String> {
    app.workspace.import_glossary(&terms, &path)
}

#[tauri::command]
fn export_glossary(app: State<App>, terms: Vec<Term>, path: PathBuf) -> Result<(), String> {
    app.workspace.export_glossary(&terms, &path)
}

/// Sends the transcript to the provider the user picked, and only that one.
#[tauri::command]
async fn summarize_job(app: State<'_, App>, id: u64, provider: Provider) -> Result<Draft, String> {
    let transcript = app.runner.transcript(id)?.text()?;
    let workspace = app.workspace.clone();
    off_thread(move || {
        workspace
            .summarize(provider, &transcript)
            .map_err(|e| e.to_string())
    })
    .await
}

/// Saves the working draft and the finished page beside the transcript; returns the page's path.
#[tauri::command]
fn save_note(
    app: State<App>,
    id: u64,
    draft: serde_json::Value,
    html: String,
) -> Result<PathBuf, String> {
    app.runner.transcript(id)?.save_note(&draft, &html)
}

#[tauri::command]
fn load_note(app: State<App>, id: u64) -> Result<Option<serde_json::Value>, String> {
    app.runner.transcript(id)?.note()
}

/// Lets the page play this job's recording, and no other file.
#[tauri::command]
fn allow_job_audio(handle: AppHandle, app: State<App>, id: u64) -> Result<PathBuf, String> {
    let source = app.runner.transcript(id)?.source().to_owned();
    handle
        .asset_protocol_scope()
        .allow_file(&source)
        .map_err(|e| e.to_string())?;
    Ok(source)
}

/// Blocking download on a worker thread; the page follows "model-progress" events.
#[tauri::command]
async fn download_models(handle: AppHandle, app: State<'_, App>) -> Result<(), String> {
    let dir = app.models_dir.clone();
    off_thread(move || {
        models::download(&dir, &|percent| {
            handle.emit("model-progress", percent).ok();
        })
    })
    .await
}

/// Keeps the Mac awake while there is work, or a long job dies when the lid timer fires.
#[derive(Default)]
struct KeepAwake(Mutex<Option<Child>>);

impl KeepAwake {
    fn update(&self, busy: bool) {
        let mut child = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match (busy, child.is_some()) {
            (true, false) => {
                // -w: caffeinate exits by itself if the app dies.
                *child = Command::new("caffeinate")
                    .args(["-i", "-w", &std::process::id().to_string()])
                    .spawn()
                    .ok();
            }
            (false, true) => {
                if let Some(mut c) = child.take() {
                    c.kill().ok();
                    c.wait().ok();
                }
            }
            _ => {}
        }
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|tauri_app| {
            let handle = tauri_app.handle().clone();
            let data = tauri_app.path().app_data_dir()?;
            let data_dir = data.clone();
            let output_dir = tauri_app.path().document_dir()?.join("ASR Studio");
            // Same folder the benchmark and any earlier download used.
            let models_dir = tauri_app
                .path()
                .data_dir()?
                .join("asr-studio")
                .join("models");

            let awake = KeepAwake::default();
            let load_dir = models_dir.clone();
            let runner = JobRunner::start(
                Paths {
                    data,
                    output: output_dir.clone(),
                },
                move || models::load_engine(&load_dir),
                move |queue: &Queue| {
                    awake.update(queue.busy);
                    handle.emit("queue", queue).ok();
                },
            )?;
            tauri_app.manage(App {
                runner,
                workspace: Workspace::at(&data_dir),
                models_dir,
                output_dir,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            enqueue,
            cancel,
            retry,
            remove,
            set_paused,
            read_transcript,
            clean_transcript,
            download_models,
            list_voice_memos,
            enqueue_voice_memos,
            open_full_disk_settings,
            relaunch,
            ai_settings,
            save_ai_settings,
            set_api_key,
            delete_api_key,
            test_provider,
            list_models,
            get_glossary,
            save_glossary,
            import_glossary,
            export_glossary,
            summarize_job,
            save_note,
            load_note,
            allow_job_audio
        ])
        .run(tauri::generate_context!())
        .expect("không mở được cửa sổ app");
}
