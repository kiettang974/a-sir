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
use crate::summary::{self, Draft};
use crate::transcriber::ResourceProfile;

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
    /// summary writing style, and any custom template columns saved at invocation time.
    pub fn summarize(&self, provider: Provider, transcript: &str) -> Result<Draft, AiError> {
        let style = self.ai_settings().summary_style;
        let columns = self.active_template_columns();
        summary::summarize_with_options(
            &self.client(provider, None)?,
            transcript,
            &self.glossary(),
            style,
            &columns,
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
