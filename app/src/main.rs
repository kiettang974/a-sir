#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio_server;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Mutex;

use audio_server::AudioServer;

use asr_core::ai::{AiSettings, Provider, SummaryStyle, SummaryStyleInfo};
use asr_core::deliverable::{MeetingRow, TemplateFormat};
use asr_core::glossary::Term;
use asr_core::models::{CustomModel, DiscoveredModel, ModelStore, ModelSummary};
use asr_core::summary::Draft;
use asr_core::transcriber::{self, ResourceProfile};
use asr_core::transcript::Line;
use asr_core::voice_memos::{self, ListError, Recording};
use asr_core::workspace::{ProfilesConfig, UpdateInfo, Workspace};
use asr_core::{Job, JobRunner, Paths, Queue, audio};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

struct App {
    runner: JobRunner,
    workspace: Workspace,
    models: ModelStore,
    output_dir: PathBuf,
    audio_server: AudioServer,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    jobs: Vec<Job>,
    groups: Vec<String>,
    paused: bool,
    busy: bool,
    models_ready: bool,
    output_dir: PathBuf,
    audio_extensions: &'static [&'static str],
    model_labels: BTreeMap<String, String>,
}

#[tauri::command]
fn snapshot(app: State<App>) -> Snapshot {
    Snapshot {
        jobs: app.runner.jobs(),
        groups: app.runner.groups(),
        paused: app.runner.is_paused(),
        busy: app.runner.is_busy(),
        models_ready: app.models.are_ready(),
        output_dir: app.output_dir.clone(),
        audio_extensions: &audio::EXTENSIONS,
        model_labels: app.models.short_labels(),
    }
}

/// Adds what it can and says why the rest were skipped.
#[tauri::command]
fn enqueue(app: State<App>, paths: Vec<PathBuf>, group: Option<String>) -> Vec<String> {
    paths
        .into_iter()
        .filter_map(|path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            app.runner
                .enqueue_in_group(path, None, group.clone())
                .err()
                .map(|e| format!("{name}: {e}"))
        })
        .collect()
}

#[tauri::command]
fn create_group(app: State<App>, name: String) -> Result<String, String> {
    app.runner.create_group(&name)
}

#[tauri::command]
fn delete_group(app: State<App>, name: String) -> Result<(), String> {
    app.runner.delete_group(&name)
}

#[tauri::command]
fn set_job_group(app: State<App>, id: u64, group: Option<String>) -> Result<(), String> {
    app.runner.set_job_group(id, group)
}

fn open_in_os(target: impl AsRef<std::ffi::OsStr>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(target.as_ref());
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]).arg(target.as_ref());
        c
    };
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(target.as_ref());
        c
    };
    cmd.status().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_group_dir(app: State<App>, name: String) -> Result<(), String> {
    let dir = app.runner.group_dir(&name)?;
    open_in_os(dir)
}

#[tauri::command]
fn open_voice_recorder() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let ok = Command::new("open")
            .args(["-b", "com.apple.VoiceMemos"])
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            return Ok(());
        }
        Command::new("open")
            .args(["-a", "Voice Memos"])
            .status()
            .map_err(|e| format!("không mở được app Ghi âm: {e}"))
            .and_then(|s| {
                if s.success() {
                    Ok(())
                } else {
                    Err("không tìm thấy app Ghi âm (Voice Memos)".into())
                }
            })
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", "ms-soundrecorder:"])
            .status()
            .map_err(|e| format!("không mở được ứng dụng Sound Recorder: {e}"))
            .and_then(|s| {
                if s.success() {
                    Ok(())
                } else {
                    Err("không mở được ứng dụng ghi âm mặc định của Windows".into())
                }
            })
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        Err("mở ứng dụng ghi âm mặc định hiện chỉ hỗ trợ macOS và Windows".into())
    }
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
    #[cfg(target_os = "macos")]
    {
        open_in_os("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
    }
    #[cfg(not(target_os = "macos"))]
    Ok(())
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
fn rerun(app: State<App>, id: u64) -> Result<u64, String> {
    app.runner.rerun(id).map_err(|e| e.to_string())
}

#[tauri::command]
fn remove(app: State<App>, id: u64) {
    app.runner.remove(id);
}

#[tauri::command]
fn clear_queue(app: State<App>) -> Vec<u64> {
    app.runner.clear_removable()
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

/// Writes the cleaned copy (with user glossary applied) beside the transcript and returns its path.
#[tauri::command]
fn clean_transcript(app: State<App>, id: u64) -> Result<PathBuf, String> {
    let terms = app.workspace.glossary();
    app.runner
        .transcript(id)?
        .write_cleaned_with_glossary(&terms)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiView {
    #[serde(flatten)]
    settings: AiSettings,
    /// A masked hint for each provider that has a key. The key itself never reaches the page.
    keys: BTreeMap<Provider, String>,
    recommended_models: BTreeMap<Provider, &'static [&'static str]>,
    summary_styles: Vec<SummaryStyleInfo>,
}

#[tauri::command]
fn ai_settings(app: State<App>) -> Result<AiView, String> {
    let recommended_models = Provider::ALL
        .iter()
        .map(|&p| (p, p.recommended_models()))
        .collect();
    Ok(AiView {
        settings: app.workspace.ai_settings(),
        keys: app.workspace.key_hints().map_err(|e| e.to_string())?,
        recommended_models,
        summary_styles: SummaryStyle::catalog(),
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
    off_thread(move || {
        client
            .list_models()
            .map(|models| asr_core::ai::filter_modern_models(provider, models))
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
async fn probe_accessible_models(
    app: State<'_, App>,
    provider: Provider,
    candidates: Option<Vec<String>>,
) -> Result<Vec<String>, String> {
    let client = app
        .workspace
        .client(provider, Some(""))
        .map_err(|e| e.to_string())?;
    off_thread(move || {
        client
            .probe_accessible_models(candidates.as_deref())
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
fn summary_profiles(app: State<App>) -> ProfilesConfig {
    app.workspace.summary_profiles()
}

#[tauri::command]
fn set_profiles_enabled(app: State<App>, enabled: bool) -> Result<ProfilesConfig, String> {
    app.workspace.set_profiles_enabled(enabled)
}

#[tauri::command]
fn select_summary_profile(app: State<App>, id: String) -> Result<ProfilesConfig, String> {
    app.workspace.select_summary_profile(&id)
}

#[tauri::command]
async fn import_summary_profile(
    app: State<'_, App>,
    path: PathBuf,
) -> Result<ProfilesConfig, String> {
    let workspace = app.workspace.clone();
    off_thread(move || workspace.import_summary_profile(&path)).await
}

#[tauri::command]
fn delete_summary_profile(app: State<App>, id: String) -> Result<ProfilesConfig, String> {
    app.workspace.delete_summary_profile(&id)
}

#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

#[tauri::command]
async fn check_update() -> Result<UpdateInfo, String> {
    off_thread(|| {
        asr_core::workspace::check_github_update(env!("CARGO_PKG_VERSION"), "kiettang974/a-sir")
    })
    .await
}

/// Downloads the installer to the user's Downloads folder and opens it.
#[tauri::command]
async fn install_update(handle: AppHandle, url: String) -> Result<String, String> {
    let dir = handle.path().download_dir().map_err(|e| e.to_string())?;
    let saved = off_thread(move || asr_core::workspace::download_update(&url, &dir)).await?;
    open_in_os(&saved)?;
    Ok(saved.display().to_string())
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("chỉ mở được liên kết https://".into());
    }
    open_in_os(url)
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MeetingTemplatesView {
    enabled: bool,
    excel: Option<String>,
    docx: Option<String>,
    excel_columns: Vec<String>,
    docx_columns: Vec<String>,
}

fn meeting_templates_view(app: &App) -> MeetingTemplatesView {
    MeetingTemplatesView {
        enabled: app.workspace.templates_enabled(),
        excel: app.workspace.template_name(TemplateFormat::Excel),
        docx: app.workspace.template_name(TemplateFormat::Docx),
        excel_columns: app.workspace.template_columns(TemplateFormat::Excel),
        docx_columns: app.workspace.template_columns(TemplateFormat::Docx),
    }
}

#[tauri::command]
fn meeting_templates(app: State<App>) -> MeetingTemplatesView {
    meeting_templates_view(&app)
}

#[tauri::command]
fn set_templates_enabled(app: State<App>, enabled: bool) -> Result<MeetingTemplatesView, String> {
    app.workspace.set_templates_enabled(enabled)?;
    Ok(meeting_templates_view(&app))
}

#[tauri::command]
fn import_meeting_template(
    app: State<App>,
    path: PathBuf,
) -> Result<MeetingTemplatesView, String> {
    app.workspace.import_template(&path)?;
    Ok(meeting_templates_view(&app))
}

#[tauri::command]
fn reset_meeting_template(
    app: State<App>,
    kind: TemplateFormat,
) -> Result<MeetingTemplatesView, String> {
    app.workspace.reset_template(kind)?;
    Ok(meeting_templates_view(&app))
}

#[tauri::command]
fn export_note_excel(
    app: State<App>,
    rows: Vec<MeetingRow>,
    path: PathBuf,
) -> Result<PathBuf, String> {
    app.workspace.export_excel(&rows, &path)?;
    Ok(path)
}

#[tauri::command]
fn export_note_docx(
    app: State<App>,
    title: String,
    greeting: String,
    intro: String,
    rows: Vec<MeetingRow>,
    path: PathBuf,
) -> Result<PathBuf, String> {
    app.workspace
        .export_docx(&title, &greeting, &intro, &rows, &path)?;
    Ok(path)
}

/// Lets the page play this job's recording, and no other file.
#[tauri::command]
fn allow_job_audio(app: State<App>, id: u64) -> Result<String, String> {
    let source = app.runner.transcript(id)?.source().to_owned();
    if let Err(e) = std::fs::File::open(&source) {
        return Err(match e.kind() {
            std::io::ErrorKind::NotFound => {
                "Không tìm thấy file ghi âm gốc (file có thể đã bị xoá hoặc di chuyển).".into()
            }
            std::io::ErrorKind::PermissionDenied => {
                "Không có quyền mở file ghi âm (nếu là bản ghi từ app Ghi âm, hãy cấp quyền Truy cập toàn bộ ổ đĩa trong Cài đặt hệ thống rồi mở lại app).".into()
            }
            _ => format!("Không mở được file ghi âm: {e}"),
        });
    }
    Ok(app.audio_server.register(id, source))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SystemInfo {
    os: &'static str,
    arch: &'static str,
    chip: String,
    ram_gb: u64,
    cpu_cores: usize,
}

fn current_os_id() -> &'static str {
    if cfg!(target_os = "macos") {
        "mac"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

fn detect_system_info() -> &'static SystemInfo {
    static INFO: std::sync::OnceLock<SystemInfo> = std::sync::OnceLock::new();
    INFO.get_or_init(|| {
        let cpu_cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0);
        let (chip, ram_gb) = detect_hw();
        SystemInfo {
            os: current_os_id(),
            arch: std::env::consts::ARCH,
            chip,
            ram_gb,
            cpu_cores,
        }
    })
}

#[cfg(target_os = "macos")]
fn detect_hw() -> (String, u64) {
    let chip = Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .unwrap_or_default();
    let ram_bytes = Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let ram_gb = (ram_bytes + 536_870_912) / 1_073_741_824;
    (chip, ram_gb)
}

#[cfg(target_os = "windows")]
fn detect_hw() -> (String, u64) {
    #[repr(C)]
    struct MemoryStatusEx {
        dw_length: u32,
        dw_memory_load: u32,
        ull_total_phys: u64,
        ull_avail_phys: u64,
        ull_total_page_file: u64,
        ull_avail_page_file: u64,
        ull_total_virtual: u64,
        ull_avail_virtual: u64,
        ull_avail_extended_virtual: u64,
    }
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(lp_buffer: *mut MemoryStatusEx) -> i32;
    }
    let mut status = MemoryStatusEx {
        dw_length: std::mem::size_of::<MemoryStatusEx>() as u32,
        dw_memory_load: 0,
        ull_total_phys: 0,
        ull_avail_phys: 0,
        ull_total_page_file: 0,
        ull_avail_page_file: 0,
        ull_total_virtual: 0,
        ull_avail_virtual: 0,
        ull_avail_extended_virtual: 0,
    };
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) } != 0;
    let ram_gb = if ok {
        (status.ull_total_phys + 536_870_912) / 1_073_741_824
    } else {
        0
    };
    let chip = std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_default();
    (chip, ram_gb)
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn detect_hw() -> (String, u64) {
    let ram_kb = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|n| n.parse::<u64>().ok())
        })
        .unwrap_or(0);
    let ram_gb = (ram_kb + 524_288) / 1_048_576;
    (String::new(), ram_gb)
}

fn min_ram_for_model(bytes: u64) -> u64 {
    if bytes <= 600_000_000 {
        4
    } else if bytes <= 2_000_000_000 {
        8
    } else {
        16
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalModelView {
    #[serde(flatten)]
    summary: ModelSummary,
    selected: bool,
    min_ram_gb: u64,
    compatible: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalModelsView {
    selected: String,
    resource_profile: ResourceProfile,
    system: SystemInfo,
    models: Vec<LocalModelView>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalModelProgress {
    model_id: String,
    percent: u8,
}

fn local_models_view(app: &App) -> LocalModelsView {
    let selected = app.workspace.asr_model();
    let system = detect_system_info().clone();
    let models = app
        .models
        .list()
        .into_iter()
        .map(|summary| {
            let min_ram_gb = min_ram_for_model(summary.bytes);
            let os_ok = summary.platforms.contains(&system.os);
            let ram_ok = system.ram_gb == 0 || system.ram_gb >= min_ram_gb;
            LocalModelView {
                selected: summary.id == selected,
                min_ram_gb,
                compatible: os_ok && ram_ok,
                summary,
            }
        })
        .collect();
    LocalModelsView {
        selected,
        resource_profile: app.workspace.resource_profile(),
        system,
        models,
    }
}

#[tauri::command]
fn local_models(app: State<App>) -> LocalModelsView {
    local_models_view(&app)
}

#[tauri::command]
async fn discover_hf_models(
    app: State<'_, App>,
    query: String,
) -> Result<Vec<DiscoveredModel>, String> {
    let store = app.models.clone();
    off_thread(move || store.discover_hf(&query)).await
}

#[tauri::command]
fn add_custom_model(app: State<App>, model: CustomModel) -> Result<LocalModelsView, String> {
    app.models.add_custom_model(model)?;
    Ok(local_models_view(&app))
}

#[tauri::command]
fn open_hf_asr_hub() -> Result<(), String> {
    open_in_os("https://huggingface.co/models?pipeline_tag=automatic-speech-recognition&sort=trending")
}

#[tauri::command]
fn open_models_dir(app: State<App>) -> Result<(), String> {
    let dir = app.models.dir();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    open_in_os(dir)
}

#[tauri::command]
async fn download_local_model(
    handle: AppHandle,
    app: State<'_, App>,
    model_id: Option<String>,
) -> Result<(), String> {
    let model_id = model_id.unwrap_or_else(|| app.workspace.asr_model());
    let store = app.models.clone();
    off_thread(move || {
        let id = model_id.clone();
        store.download(&model_id, &|percent| {
            handle
                .emit(
                    "local-model-progress",
                    LocalModelProgress {
                        model_id: id.clone(),
                        percent,
                    },
                )
                .ok();
        })
    })
    .await
}

#[tauri::command]
fn select_local_model(app: State<App>, model_id: String) -> Result<LocalModelsView, String> {
    app.models.validate_select(&model_id)?;
    app.workspace.set_asr_model(&model_id)?;
    app.runner.set_default_model(model_id);
    Ok(local_models_view(&app))
}

#[tauri::command]
fn delete_local_model(app: State<App>, model_id: String) -> Result<LocalModelsView, String> {
    app.models.delete(&model_id, &app.workspace.asr_model())?;
    Ok(local_models_view(&app))
}

#[tauri::command]
fn set_resource_profile(
    app: State<App>,
    profile: ResourceProfile,
) -> Result<LocalModelsView, String> {
    app.workspace.set_resource_profile(profile)?;
    Ok(local_models_view(&app))
}

/// Keeps the machine awake while there is work, or a long job dies when the lid timer fires.
#[derive(Default)]
struct KeepAwake(Mutex<Option<Child>>);

impl KeepAwake {
    fn update(&self, busy: bool) {
        #[cfg(target_os = "macos")]
        {
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
        #[cfg(not(target_os = "macos"))]
        let _ = (&self.0, busy);
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

            let workspace = Workspace::at(&data_dir);
            let models_store = ModelStore::new(models_dir);
            let selected_model = workspace.asr_model();
            let awake = KeepAwake::default();
            let load_store = models_store.clone();
            let load_ws = workspace.clone();
            let runner = JobRunner::start(
                Paths {
                    data,
                    output: output_dir.clone(),
                },
                selected_model,
                move |model_id| {
                    let prompt = transcriber::glossary_prompt(&load_ws.glossary());
                    load_store.load(model_id, load_ws.resource_profile(), prompt)
                },
                move |queue: &Queue| {
                    awake.update(queue.busy);
                    handle.emit("queue", queue).ok();
                },
            )?;
            let audio_server = AudioServer::start()?;
            tauri_app.manage(App {
                runner,
                workspace,
                models: models_store,
                output_dir,
                audio_server,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            enqueue,
            create_group,
            delete_group,
            set_job_group,
            open_group_dir,
            open_voice_recorder,
            cancel,
            retry,
            rerun,
            remove,
            clear_queue,
            set_paused,
            read_transcript,
            clean_transcript,
            local_models,
            discover_hf_models,
            add_custom_model,
            open_hf_asr_hub,
            open_models_dir,
            download_local_model,
            select_local_model,
            delete_local_model,
            set_resource_profile,
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
            probe_accessible_models,
            summary_profiles,
            set_profiles_enabled,
            select_summary_profile,
            import_summary_profile,
            delete_summary_profile,
            app_version,
            check_update,
            install_update,
            open_url,
            get_glossary,
            save_glossary,
            import_glossary,
            export_glossary,
            summarize_job,
            save_note,
            load_note,
            meeting_templates,
            set_templates_enabled,
            import_meeting_template,
            reset_meeting_template,
            export_note_excel,
            export_note_docx,
            allow_job_audio
        ])
        .run(tauri::generate_context!())
        .expect("không mở được cửa sổ app");
}
