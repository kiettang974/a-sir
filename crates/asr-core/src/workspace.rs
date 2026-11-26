//! The user's own setup, kept in the app's data folder: which AI provider and model to use,
//! the API keys, and the glossary of correct terms. Callers name a provider or a CSV file;
//! file names, formats and fallbacks stay in here.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ai::{AiError, AiSettings, ApiKeys, HttpClient, Provider};
use crate::deliverable::{self, MeetingRow, TemplateFormat};
use crate::docx;
use crate::excel;
use crate::glossary::{self, Term};
use crate::models;
use crate::output::save_atomic;
use crate::summary::{self, Draft, SummaryProfileMode};
use crate::transcriber::ResourceProfile;

pub const DEFAULT_PROFILE_ID: &str = "ba-open-kit";

fn default_true() -> bool {
    true
}

fn default_profile_id() -> String {
    DEFAULT_PROFILE_ID.to_owned()
}

/// User-uploaded Markdown summary profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryProfile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub content: String,
    #[serde(default)]
    pub source_file: Option<String>,
}

/// Summary profile settings persisted in `summary-profiles.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilesConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_profile_id")]
    pub active_id: String,
    #[serde(default)]
    pub custom: Vec<SummaryProfile>,
}

impl Default for ProfilesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            active_id: default_profile_id(),
            custom: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AsrSettings {
    #[serde(default = "default_model_id")]
    model_id: String,
    #[serde(default)]
    resource_profile: ResourceProfile,
}

fn default_model_id() -> String {
    models::DEFAULT_MODEL_ID.to_owned()
}

impl Default for AsrSettings {
    fn default() -> Self {
        Self {
            model_id: default_model_id(),
            resource_profile: ResourceProfile::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TemplateMeta {
    name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TemplateToggle {
    #[serde(default)]
    enabled: bool,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    dir: PathBuf,
}

impl Workspace {
    pub fn at(dir: &Path) -> Self {
        Self {
            dir: dir.to_owned(),
        }
    }

    fn asr_settings_path(&self) -> PathBuf {
        self.dir.join("asr.json")
    }

    fn settings_path(&self) -> PathBuf {
        self.dir.join("ai.json")
    }

    fn glossary_path(&self) -> PathBuf {
        self.dir.join("glossary.json")
    }

    fn template_path(&self, format: TemplateFormat) -> PathBuf {
        self.dir.join(format.storage_filename())
    }

    fn template_meta_path(&self, format: TemplateFormat) -> PathBuf {
        self.dir.join(format.meta_filename())
    }

    fn template_toggle_path(&self) -> PathBuf {
        self.dir.join("meeting-templates-toggle.json")
    }

    fn profiles_path(&self) -> PathBuf {
        self.dir.join("summary-profiles.json")
    }

    pub fn summary_profiles(&self) -> ProfilesConfig {
        fs::read(self.profiles_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save_summary_profiles(&self, cfg: &ProfilesConfig) -> Result<(), String> {
        let body = serde_json::to_vec_pretty(cfg).map_err(|e| e.to_string())?;
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        save_atomic(&self.profiles_path(), &body).map_err(|e| e.to_string())
    }

    pub fn set_profiles_enabled(&self, enabled: bool) -> Result<ProfilesConfig, String> {
        let mut cfg = self.summary_profiles();
        cfg.enabled = enabled;
        self.save_summary_profiles(&cfg)?;
        Ok(cfg)
    }

    pub fn select_summary_profile(&self, id: &str) -> Result<ProfilesConfig, String> {
        let mut cfg = self.summary_profiles();
        let valid = id == DEFAULT_PROFILE_ID || cfg.custom.iter().any(|p| p.id == id);
        if !valid {
            return Err("không tìm thấy profile này".into());
        }
        cfg.active_id = id.to_owned();
        cfg.enabled = true;
        self.save_summary_profiles(&cfg)?;
        Ok(cfg)
    }

    pub fn delete_summary_profile(&self, id: &str) -> Result<ProfilesConfig, String> {
        let mut cfg = self.summary_profiles();
        cfg.custom.retain(|p| p.id != id);
        if cfg.active_id == id {
            cfg.active_id = DEFAULT_PROFILE_ID.to_owned();
        }
        self.save_summary_profiles(&cfg)?;
        Ok(cfg)
    }

    /// Reads a user `.md` file, asks the configured AI provider to generate a concise Vietnamese
    /// profile name & 1-line description (falling back to filename/heading if offline or no key),
    /// saves it, and makes it the active profile.
    pub fn import_summary_profile(&self, path: &Path) -> Result<ProfilesConfig, String> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if ext != "md" && ext != "markdown" {
            return Err("chỉ hỗ trợ file Markdown (.md)".into());
        }
        let content = fs::read_to_string(path)
            .map_err(|e| format!("không đọc được file .md: {e}"))?;
        if content.trim().is_empty() {
            return Err("file .md đang trống".into());
        }
        let source_file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "profile.md".into());
        let fallback_name = path
            .file_stem()
            .map(|s| s.to_string_lossy().replace(['-', '_'], " ").trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Profile riêng".into());
        let fallback_desc = content
            .lines()
            .map(|l| l.trim().trim_start_matches('#').trim())
            .find(|l| !l.is_empty())
            .map(|l| l.chars().take(120).collect::<String>())
            .unwrap_or_else(|| format!("Tải lên từ {source_file}"));

        let (name, description) = self
            .name_profile_with_ai(&content)
            .unwrap_or((fallback_name, fallback_desc));

        let mut cfg = self.summary_profiles();
        let slug: String = name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(16)
            .collect::<String>()
            .to_lowercase();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let id = format!(
            "custom-{}-{stamp}",
            if slug.is_empty() { "profile" } else { &slug }
        );
        cfg.custom.push(SummaryProfile {
            id: id.clone(),
            name,
            description,
            content,
            source_file: Some(source_file),
        });
        cfg.active_id = id;
        cfg.enabled = true;
        self.save_summary_profiles(&cfg)?;
        Ok(cfg)
    }

    fn name_profile_with_ai(&self, content: &str) -> Option<(String, String)> {
        let settings = self.ai_settings();
        let mut order = vec![settings.default_provider];
        for p in Provider::ALL {
            if !order.contains(&p) {
                order.push(p);
            }
        }
        let client = order
            .into_iter()
            .find_map(|p| self.client(p, None).ok())?;
        let snippet: String = content.chars().take(3000).collect();
        let system = "Bạn đặt tên ngắn gọn tiếng Việt cho một profile hướng dẫn tóm tắt cuộc họp.\nTrả lời ĐÚNG 2 dòng, không giải thích thêm:\nNAME: <tên profile ngắn 2-5 từ>\nDESC: <mô tả 1 câu ngắn dưới 18 từ>";
        let reply = client.quick_text(system, &snippet, 120).ok()?;
        let mut name = None;
        let mut desc = None;
        for line in reply.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("NAME:") {
                let v = rest.trim().trim_matches('"').to_owned();
                if !v.is_empty() {
                    name = Some(v);
                }
            } else if let Some(rest) = trimmed.strip_prefix("DESC:") {
                let v = rest.trim().trim_matches('"').to_owned();
                if !v.is_empty() {
                    desc = Some(v);
                }
            }
        }
        Some((name?, desc.unwrap_or_else(|| "Profile tóm tắt riêng từ file Markdown.".into())))
    }

    pub fn templates_enabled(&self) -> bool {
        fs::read(self.template_toggle_path())
            .ok()
            .and_then(|b| serde_json::from_slice::<TemplateToggle>(&b).ok())
            .is_some_and(|t| t.enabled)
    }

    pub fn set_templates_enabled(&self, enabled: bool) -> Result<(), String> {
        let body = serde_json::to_vec(&TemplateToggle { enabled }).map_err(|e| e.to_string())?;
        save_atomic(&self.template_toggle_path(), &body)
            .map_err(|e| format!("không lưu được trạng thái template: {e}"))
    }

    fn keys(&self) -> ApiKeys {
        ApiKeys::at(&self.dir.join("api-keys.json"))
    }

    /// Original filename of the imported custom template for `format`, if one is set.
    pub fn template_name(&self, format: TemplateFormat) -> Option<String> {
        if !self.template_path(format).exists() {
            return None;
        }
        fs::read(self.template_meta_path(format))
            .ok()
            .and_then(|b| serde_json::from_slice::<TemplateMeta>(&b).ok())
            .map(|m| m.name)
            .or_else(|| Some(format.storage_filename().to_owned()))
    }

    /// Validates and saves a user-provided `.xlsx` or `.docx` file as the meeting export template.
    pub fn import_template(&self, path: &Path) -> Result<(TemplateFormat, String), String> {
        let format = TemplateFormat::from_path(path)?;
        let bytes = fs::read(path).map_err(|e| format!("không đọc được file mẫu: {e}"))?;
        deliverable::validate_template(format, &bytes)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format.storage_filename().to_owned());
        save_atomic(&self.template_path(format), &bytes)
            .map_err(|e| format!("không lưu được file mẫu: {e}"))?;
        let meta = serde_json::to_vec(&TemplateMeta { name: name.clone() })
            .map_err(|e| e.to_string())?;
        save_atomic(&self.template_meta_path(format), &meta)
            .map_err(|e| format!("không lưu được thông tin file mẫu: {e}"))?;
        let _ = self.set_templates_enabled(true);
        Ok((format, name))
    }

    /// Removes the custom template for `format` so exports fall back to the built-in layout.
    pub fn reset_template(&self, format: TemplateFormat) -> Result<(), String> {
        let _ = fs::remove_file(self.template_path(format));
        let _ = fs::remove_file(self.template_meta_path(format));
        Ok(())
    }

    /// Exports confirmed rows to `.xlsx`, using the imported custom template when enabled.
    pub fn export_excel(&self, rows: &[MeetingRow], out_path: &Path) -> Result<(), String> {
        let custom = self
            .templates_enabled()
            .then(|| fs::read(self.template_path(TemplateFormat::Excel)).ok())
            .flatten();
        excel::export(custom.as_deref(), rows, out_path)
    }

    /// Exports confirmed rows to `.docx`, using the imported custom Word template when enabled.
    pub fn export_docx(
        &self,
        title: &str,
        greeting: &str,
        intro: &str,
        rows: &[MeetingRow],
        out_path: &Path,
    ) -> Result<(), String> {
        let custom = self
            .templates_enabled()
            .then(|| fs::read(self.template_path(TemplateFormat::Docx)).ok())
            .flatten();
        docx::export(custom.as_deref(), title, greeting, intro, rows, out_path)
    }

    fn load_asr_settings(&self) -> AsrSettings {
        fs::read(self.asr_settings_path())
            .ok()
            .and_then(|b| serde_json::from_slice::<AsrSettings>(&b).ok())
            .unwrap_or_default()
    }

    fn save_asr_settings(&self, settings: &AsrSettings) -> Result<(), String> {
        let body = serde_json::to_vec(settings).map_err(|e| e.to_string())?;
        save_atomic(&self.asr_settings_path(), &body)
            .map_err(|e| format!("không lưu được cấu hình chép lời: {e}"))
    }

    /// The selected local transcription model ID; falls back to the default catalog model.
    pub fn asr_model(&self) -> String {
        let id = self.load_asr_settings().model_id;
        if id.trim().is_empty() {
            models::DEFAULT_MODEL_ID.to_owned()
        } else {
            id
        }
    }

    pub fn set_asr_model(&self, model_id: &str) -> Result<(), String> {
        let trimmed = model_id.trim();
        if trimmed.is_empty() {
            return Err("mã model chép lời không được để trống".into());
        }
        let mut settings = self.load_asr_settings();
        settings.model_id = trimmed.to_owned();
        self.save_asr_settings(&settings)
    }

    pub fn resource_profile(&self) -> ResourceProfile {
        self.load_asr_settings().resource_profile
    }

    pub fn set_resource_profile(&self, profile: ResourceProfile) -> Result<(), String> {
        let mut settings = self.load_asr_settings();
        settings.resource_profile = profile;
        self.save_asr_settings(&settings)
    }

    /// The settings in effect: a provider whose model was left blank shows its default,
    /// which is also what a request to it would use.
    pub fn ai_settings(&self) -> AiSettings {
        let mut settings = AiSettings::load(&self.settings_path());
        for p in Provider::ALL {
            let model = settings.model(p).to_owned();
            settings.models.insert(p, model);
        }
        settings
    }

    pub fn save_ai_settings(&self, settings: &AiSettings) -> Result<(), String> {
        settings.save(&self.settings_path())
    }

    /// A masked hint for each provider that has a key. The key itself never leaves.
    pub fn key_hints(&self) -> Result<BTreeMap<Provider, String>, AiError> {
        self.keys().hints()
    }

    pub fn set_key(&self, provider: Provider, key: &str) -> Result<(), AiError> {
        self.keys().set(provider, key)
    }

    pub fn delete_key(&self, provider: Provider) -> Result<(), AiError> {
        self.keys().remove(provider)
    }

    /// A client for `provider` with its stored key, using `model`, or the configured model
    /// when `None`.
    pub fn client(&self, provider: Provider, model: Option<&str>) -> Result<HttpClient, AiError> {
        let key = self.keys().get(provider)?.ok_or(AiError::MissingKey)?;
        let model = match model {
            Some(m) => m.to_owned(),
            None => self.ai_settings().model(provider).to_owned(),
        };
        Ok(HttpClient::new(provider, &model, &key))
    }

    pub fn template_columns(&self, format: TemplateFormat) -> Vec<String> {
        fs::read(self.template_path(format))
            .ok()
            .and_then(|b| deliverable::inspect_headers(format, &b).ok())
            .unwrap_or_default()
    }

    /// Combines unique column headers from any imported custom Excel/Word template when
    /// templates are enabled; returns empty when toggled off so AI runs pure BA-Open-Kit + Humanizer.
    pub fn active_template_columns(&self) -> Vec<String> {
        if !self.templates_enabled() {
            return Vec::new();
        }
        let mut cols = self.template_columns(TemplateFormat::Excel);
        for c in self.template_columns(TemplateFormat::Docx) {
            if !cols.iter().any(|existing| existing.eq_ignore_ascii_case(&c)) {
                cols.push(c);
            }
        }
        cols
    }

    /// Sends the transcript to `provider`, and only that one, with the user's glossary,
    /// summary writing style, active summary profile, and any custom template columns saved at invocation time.
    pub fn summarize(&self, provider: Provider, transcript: &str) -> Result<Draft, AiError> {
        let style = self.ai_settings().summary_style;
        let columns = self.active_template_columns();
        let profiles = self.summary_profiles();
        let custom_profile = if profiles.enabled && profiles.active_id != DEFAULT_PROFILE_ID {
            profiles
                .custom
                .iter()
                .find(|p| p.id == profiles.active_id)
        } else {
            None
        };
        let profile_mode = if !profiles.enabled {
            SummaryProfileMode::Disabled
        } else if let Some(p) = custom_profile {
            SummaryProfileMode::Custom {
                name: &p.name,
                content: &p.content,
            }
        } else {
            SummaryProfileMode::BaOpenKit
        };
        summary::summarize_with_profile(
            &self.client(provider, None)?,
            transcript,
            &self.glossary(),
            style,
            &columns,
            profile_mode,
        )
    }

    pub fn glossary(&self) -> Vec<Term> {
        glossary::load(&self.glossary_path())
    }

    pub fn save_glossary(&self, terms: &[Term]) -> Result<(), String> {
        glossary::save(&self.glossary_path(), terms)
    }

    /// Merges a CSV file into `current` (the list as the user sees it now), saves the result
    /// and returns it. Taking `current` means an edit not yet saved is never lost.
    pub fn import_glossary(&self, current: &[Term], csv: &Path) -> Result<Vec<Term>, String> {
        let text = fs::read_to_string(csv).map_err(|e| format!("không đọc được file: {e}"))?;
        let merged = glossary::merge(current, &glossary::from_csv(&text)?);
        self.save_glossary(&merged)?;
        Ok(self.glossary())
    }

    /// Saves `current` and writes it to a CSV file.
    pub fn export_glossary(&self, current: &[Term], csv: &Path) -> Result<(), String> {
        self.save_glossary(current)?;
        let text = glossary::to_csv(&self.glossary())?;
        fs::write(csv, text).map_err(|e| format!("không ghi được file: {e}"))
    }
}

/// Latest GitHub release metadata compared against the running app version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    pub release_url: String,
    pub download_url: Option<String>,
    pub notes: String,
}

fn parse_semver(v: &str) -> (u64, u64, u64) {
    let clean = v.trim().trim_start_matches(['v', 'V']);
    let mut parts = clean.split('.').map(|p| {
        p.chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<u64>()
            .unwrap_or(0)
    });
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// Checks GitHub Releases for `repo` (e.g. `"kiettang974/a-sir"`) and returns update info.
pub fn check_github_update(current_version: &str, repo: &str) -> Result<UpdateInfo, String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(4))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("không khởi tạo được kết nối: {e}"))?;
    let resp = client
        .get(&url)
        .header("User-Agent", "A-Sir")
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|e| format!("không kết nối được GitHub: {e}"))?;
    if resp.status().as_u16() == 404 {
        return Ok(UpdateInfo {
            current_version: current_version.to_owned(),
            latest_version: current_version.to_owned(),
            has_update: false,
            release_url: format!("https://github.com/{repo}/releases"),
            download_url: None,
            notes: "Chưa có bản phát hành nào trên GitHub Releases.".into(),
        });
    }
    if !resp.status().is_success() {
        return Err(format!("GitHub trả lỗi HTTP {}", resp.status().as_u16()));
    }
    let val: serde_json::Value = resp
        .json()
        .map_err(|e| format!("dữ liệu phản hồi không hợp lệ: {e}"))?;
    let tag = val["tag_name"]
        .as_str()
        .unwrap_or(current_version)
        .trim_start_matches(['v', 'V'])
        .to_owned();
    let release_url = val["html_url"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("https://github.com/{repo}/releases"));
    let notes = val["body"].as_str().unwrap_or_default().trim().to_owned();

    let preferred_exts: &[&str] = if cfg!(target_os = "macos") {
        &[".dmg", ".app.tar.gz"]
    } else if cfg!(target_os = "windows") {
        &[".exe", ".msi"]
    } else {
        &[".AppImage", ".deb"]
    };
    let download_url = val["assets"].as_array().and_then(|assets| {
        preferred_exts.iter().find_map(|ext| {
            assets.iter().find_map(|a| {
                let name = a["name"].as_str()?;
                if name.to_ascii_lowercase().ends_with(&ext.to_ascii_lowercase()) {
                    a["browser_download_url"].as_str().map(str::to_owned)
                } else {
                    None
                }
            })
        })
    });

    let has_update = parse_semver(&tag) > parse_semver(current_version);
    Ok(UpdateInfo {
        current_version: current_version.to_owned(),
        latest_version: tag,
        has_update,
        release_url,
        download_url,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(correct: &str, heard_as: &str) -> Term {
        Term {
            correct: correct.into(),
            heard_as: heard_as.into(),
            note: String::new(),
        }
    }

    #[test]
    fn a_blank_model_shows_the_default_that_requests_would_use() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::at(dir.path());
        let mut settings = ws.ai_settings();
        settings.models.insert(Provider::Gemini, "  ".into());
        settings.models.insert(Provider::Claude, "claude-sonnet-5-5".into());
        ws.save_ai_settings(&settings).unwrap();
        let shown = ws.ai_settings();
        assert_eq!(shown.models[&Provider::Gemini], Provider::Gemini.default_model());
        assert_eq!(shown.models[&Provider::Claude], "claude-sonnet-5-5");
    }

    #[test]
    fn no_key_means_no_client_and_no_summary() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::at(dir.path());
        assert_eq!(ws.client(Provider::Claude, None).err(), Some(AiError::MissingKey));
        assert_eq!(ws.summarize(Provider::Claude, "x").err(), Some(AiError::MissingKey));
        ws.set_key(Provider::Claude, "sk-ant-1234567890").unwrap();
        assert!(ws.client(Provider::Claude, None).is_ok());
        assert!(ws.key_hints().unwrap().contains_key(&Provider::Claude));
        ws.delete_key(Provider::Claude).unwrap();
        assert_eq!(ws.client(Provider::Claude, None).err(), Some(AiError::MissingKey));
    }

    #[test]
    fn import_merges_into_the_list_on_screen_even_if_it_was_not_saved() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::at(dir.path());
        ws.save_glossary(&[term("ERP", "cũ")]).unwrap();
        let on_screen = [term("ERP", "cũ"), term("LCN", "chưa lưu")];
        let csv = dir.path().join("t.csv");
        fs::write(&csv, "Viết đúng,Hay bị nghe thành,Giải thích\nERP,mới,\nBravo,ba vô,\n").unwrap();
        let merged = ws.import_glossary(&on_screen, &csv).unwrap();
        assert_eq!(
            merged,
            vec![term("ERP", "mới"), term("LCN", "chưa lưu"), term("Bravo", "ba vô")]
        );
        assert_eq!(ws.glossary(), merged, "saved too");
    }

    #[test]
    fn export_writes_the_list_on_screen_and_saves_it() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::at(dir.path());
        let csv = dir.path().join("out.csv");
        ws.export_glossary(&[term("LCN", "el xi en")], &csv).unwrap();
        assert_eq!(
            glossary::from_csv(&fs::read_to_string(&csv).unwrap()).unwrap(),
            vec![term("LCN", "el xi en")]
        );
        assert_eq!(ws.glossary(), vec![term("LCN", "el xi en")]);
    }

    #[test]
    fn a_missing_csv_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::at(dir.path());
        let error = ws.import_glossary(&[], &dir.path().join("không có.csv")).unwrap_err();
        assert!(error.contains("không đọc được"));
    }

    #[test]
    fn asr_model_persists_across_reloads_and_rejects_blank_ids() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::at(dir.path());
        assert_eq!(ws.asr_model(), models::DEFAULT_MODEL_ID);
        assert!(ws.set_asr_model("   ").is_err());
        ws.set_asr_model("phowhisper-medium-q5_0").unwrap();
        assert_eq!(Workspace::at(dir.path()).asr_model(), "phowhisper-medium-q5_0");
    }

    #[test]
    fn templates_import_inspect_and_reset_for_both_formats() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::at(dir.path());

        let xlsx = dir.path().join("custom.xlsx");
        excel::export(None, &[], &xlsx).unwrap();
        let (fmt, name) = ws.import_template(&xlsx).unwrap();
        assert_eq!(fmt, TemplateFormat::Excel);
        assert_eq!(name, "custom.xlsx");
        assert_eq!(ws.template_name(TemplateFormat::Excel).as_deref(), Some("custom.xlsx"));
        assert!(!ws.template_columns(TemplateFormat::Excel).is_empty());

        let docx_path = dir.path().join("custom.docx");
        docx::export(None, "Họp", "", "", &[], &docx_path).unwrap();
        let (fmt, name) = ws.import_template(&docx_path).unwrap();
        assert_eq!(fmt, TemplateFormat::Docx);
        assert_eq!(name, "custom.docx");
        assert_eq!(ws.template_name(TemplateFormat::Docx).as_deref(), Some("custom.docx"));
        assert!(!ws.template_columns(TemplateFormat::Docx).is_empty());

        ws.reset_template(TemplateFormat::Excel).unwrap();
        assert_eq!(ws.template_name(TemplateFormat::Excel), None);
        assert_eq!(ws.template_name(TemplateFormat::Docx).as_deref(), Some("custom.docx"));

        let bad = dir.path().join("notes.txt");
        fs::write(&bad, "hi").unwrap();
        assert!(ws.import_template(&bad).is_err());
    }
}
