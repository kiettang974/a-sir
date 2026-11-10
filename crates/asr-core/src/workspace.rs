//! The user's own setup, kept in the app's data folder: which AI provider and model to use,
//! the API keys, and the glossary of correct terms. Callers name a provider or a CSV file;
//! file names, formats and fallbacks stay in here.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::ai::{AiError, AiSettings, ApiKeys, HttpClient, Provider};
use crate::glossary::{self, Term};
use crate::summary::{self, Draft};

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

    fn settings_path(&self) -> PathBuf {
        self.dir.join("ai.json")
    }

    fn glossary_path(&self) -> PathBuf {
        self.dir.join("glossary.json")
    }

    fn keys(&self) -> ApiKeys {
        ApiKeys::at(&self.dir.join("api-keys.json"))
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

    /// Sends the transcript to `provider`, and only that one, with the user's glossary.
    pub fn summarize(&self, provider: Provider, transcript: &str) -> Result<Draft, AiError> {
        summary::summarize(&self.client(provider, None)?, transcript, &self.glossary())
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
}
