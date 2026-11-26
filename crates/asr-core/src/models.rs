//! The speech engine's model catalog: checking what is downloaded, fetching single- or
//! multi-file models with resume + SHA-256 verification, and loading the requested engine.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::output::save_atomic;
use crate::transcriber::{
    QwenAsr, ResourceProfile, TranscribeError, Transcriber, WhisperCpp, WhisperOptions,
};

pub const DEFAULT_MODEL_ID: &str = "whisper-large-v3-turbo";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    WhisperCpp,
    QwenAsr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelAsset {
    pub file: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub short_name: &'static str,
    pub description: &'static str,
    pub source: &'static str,
    pub engine: &'static str,
    pub engine_kind: EngineKind,
    /// Optional subdirectory inside `models_dir` for multi-file models (e.g. Qwen3-ASR).
    pub subdir: Option<&'static str>,
    pub files: &'static [ModelAsset],
}

const WHISPER_TURBO_FILES: &[ModelAsset] = &[ModelAsset {
    file: "ggml-large-v3-turbo.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin",
    sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
    bytes: 1_624_555_275,
}];

const WHISPER_FULL_FILES: &[ModelAsset] = &[ModelAsset {
    file: "ggml-large-v3.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin",
    sha256: "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2",
    bytes: 3_095_033_483,
}];

const WHISPER_FULL_Q5_FILES: &[ModelAsset] = &[ModelAsset {
    file: "ggml-large-v3-q5_0.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-q5_0.bin",
    sha256: "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1",
    bytes: 1_081_140_203,
}];

const WHISPER_TURBO_Q5_FILES: &[ModelAsset] = &[ModelAsset {
    file: "ggml-large-v3-turbo-q5_0.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
    sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
    bytes: 574_041_195,
}];

const WHISPER_MED_FILES: &[ModelAsset] = &[ModelAsset {
    file: "ggml-medium.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
    sha256: "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
    bytes: 1_533_763_059,
}];

const WHISPER_SMALL_FILES: &[ModelAsset] = &[ModelAsset {
    file: "ggml-small.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
    sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
    bytes: 487_601_967,
}];

const PHOWHISPER_MED_FILES: &[ModelAsset] = &[ModelAsset {
    file: "ggml-phowhisper-medium-q5_0.bin",
    url: "https://huggingface.co/nhgpv11/PhoWhisper-ggml/resolve/main/ggml-phowhisper-medium-q5_0.bin",
    sha256: "e2cd4ceda28646fc74e48f171fad8ebd75e0ab06817d70349bc42ae2335bfc27",
    bytes: 539_212_484,
}];

const PHOWHISPER_SMALL_FILES: &[ModelAsset] = &[ModelAsset {
    file: "ggml-phowhisper-small-q5_0.bin",
    url: "https://huggingface.co/nhgpv11/PhoWhisper-ggml/resolve/main/ggml-phowhisper-small-q5_0.bin",
    sha256: "4ecf5391f52b62029284300ccbfdf9dd420c1af1b144acbb1be5d22e62e24496",
    bytes: 175_209_680,
}];

const QWEN3_ASR_1_7B_FILES: &[ModelAsset] = &[
    ModelAsset {
        file: "config.json",
        url: "https://huggingface.co/Qwen/Qwen3-ASR-1.7B/resolve/main/config.json",
        sha256: "2e74a751548b8ad7d7526d29365ad8144c345d8b412b1152d25dc6698452712f",
        bytes: 6_194,
    },
    ModelAsset {
        file: "model.safetensors.index.json",
        url: "https://huggingface.co/Qwen/Qwen3-ASR-1.7B/resolve/main/model.safetensors.index.json",
        sha256: "f994739fe38e5210b9e3e8ce6c6307315e2ceac3cb630e7b7414d69dce520f60",
        bytes: 64_821,
    },
    ModelAsset {
        file: "vocab.json",
        url: "https://huggingface.co/Qwen/Qwen3-ASR-1.7B/resolve/main/vocab.json",
        sha256: "ca10d7e9fb3ed18575dd1e277a2579c16d108e32f27439684afa0e10b1440910",
        bytes: 2_776_833,
    },
    ModelAsset {
        file: "merges.txt",
        url: "https://huggingface.co/Qwen/Qwen3-ASR-1.7B/resolve/main/merges.txt",
        sha256: "8831e4f1a044471340f7c0a83d7bd71306a5b867e95fd870f74d0c5308a904d5",
        bytes: 1_671_853,
    },
    ModelAsset {
        file: "model-00001-of-00002.safetensors",
        url: "https://huggingface.co/Qwen/Qwen3-ASR-1.7B/resolve/main/model-00001-of-00002.safetensors",
        sha256: "a4cd1f1a04d90b757dc7f7dd26254e69a013b19e80efe590a83c6a3bde8608d6",
        bytes: 4_220_320_824,
    },
    ModelAsset {
        file: "model-00002-of-00002.safetensors",
        url: "https://huggingface.co/Qwen/Qwen3-ASR-1.7B/resolve/main/model-00002-of-00002.safetensors",
        sha256: "6e0b9d9e09e2e0238e7ef3cc8a484ab387e91b90f1900bedf88bc92d7929ccfc",
        bytes: 478_200_688,
    },
];

pub const CATALOG: &[ModelSpec] = &[
    ModelSpec {
        id: "whisper-large-v3-turbo",
        name: "Whisper Large V3 Turbo",
        short_name: "Turbo",
        description: "Mặc định, cân bằng tốc độ và độ chính xác tiếng Việt và tiếng Anh.",
        source: "ggerganov/whisper.cpp",
        engine: "whisper.cpp",
        engine_kind: EngineKind::WhisperCpp,
        subdir: None,
        files: WHISPER_TURBO_FILES,
    },
    ModelSpec {
        id: "whisper-large-v3",
        name: "Whisper Large V3 (Full)",
        short_name: "Large V3 Full",
        description: "Độ chính xác cao cho cuộc họp dài và thuật ngữ tiếng Anh, cần nhiều bộ nhớ hơn.",
        source: "ggerganov/whisper.cpp",
        engine: "whisper.cpp",
        engine_kind: EngineKind::WhisperCpp,
        subdir: None,
        files: WHISPER_FULL_FILES,
    },
    ModelSpec {
        id: "whisper-large-v3-q5_0",
        name: "Whisper Large V3 (Q5_0)",
        short_name: "Large V3 Q5",
        description: "Bản nén nhẹ hơn của Large V3, giữ độ chính xác tốt và tiết kiệm RAM.",
        source: "ggerganov/whisper.cpp",
        engine: "whisper.cpp",
        engine_kind: EngineKind::WhisperCpp,
        subdir: None,
        files: WHISPER_FULL_Q5_FILES,
    },
    ModelSpec {
        id: "whisper-large-v3-turbo-q5_0",
        name: "Whisper Large V3 Turbo (Q5_0)",
        short_name: "Turbo Q5",
        description: "Bản nén siêu nhanh của Turbo, nhẹ máy và phù hợp bản ghi dài.",
        source: "ggerganov/whisper.cpp",
        engine: "whisper.cpp",
        engine_kind: EngineKind::WhisperCpp,
        subdir: None,
        files: WHISPER_TURBO_Q5_FILES,
    },
    ModelSpec {
        id: "qwen3-asr-1.7b",
        name: "Qwen3-ASR-1.7B",
        short_name: "Qwen3-1.7B",
        description: "Mạnh về tiếng Việt và hội thoại pha trộn Anh–Việt.",
        source: "Qwen/Qwen3-ASR-1.7B",
        engine: "qwen-asr",
        engine_kind: EngineKind::QwenAsr,
        subdir: Some("qwen3-asr-1.7b"),
        files: QWEN3_ASR_1_7B_FILES,
    },
    ModelSpec {
        id: "phowhisper-medium-q5_0",
        name: "PhoWhisper Medium",
        short_name: "PhoWhisper Med",
        description: "Model tinh chỉnh chuyên cho tiếng Việt.",
        source: "nhgpv11/PhoWhisper-ggml",
        engine: "whisper.cpp",
        engine_kind: EngineKind::WhisperCpp,
        subdir: None,
        files: PHOWHISPER_MED_FILES,
    },
    ModelSpec {
        id: "phowhisper-small-q5_0",
        name: "PhoWhisper Small",
        short_name: "PhoWhisper Sm",
        description: "Bản gọn nhẹ chuyên tiếng Việt, tải nhanh và ít tốn tài nguyên.",
        source: "nhgpv11/PhoWhisper-ggml",
        engine: "whisper.cpp",
        engine_kind: EngineKind::WhisperCpp,
        subdir: None,
        files: PHOWHISPER_SMALL_FILES,
    },
    ModelSpec {
        id: "whisper-medium",
        name: "Whisper Medium",
        short_name: "Medium",
        description: "Model đa ngôn ngữ cỡ trung, ổn định cho tiếng Việt và tiếng Anh.",
        source: "ggerganov/whisper.cpp",
        engine: "whisper.cpp",
        engine_kind: EngineKind::WhisperCpp,
        subdir: None,
        files: WHISPER_MED_FILES,
    },
    ModelSpec {
        id: "whisper-small",
        name: "Whisper Small",
        short_name: "Small",
        description: "Nhẹ và nhanh, phù hợp kiểm tra nhanh hoặc máy ít RAM.",
        source: "ggerganov/whisper.cpp",
        engine: "whisper.cpp",
        engine_kind: EngineKind::WhisperCpp,
        subdir: None,
        files: WHISPER_SMALL_FILES,
    },
];

const VAD: ModelAsset = ModelAsset {
    file: "ggml-silero-v5.1.2.bin",
    url: "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin",
    sha256: "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf",
    bytes: 885_098,
};

impl ModelSpec {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }

    pub fn model_dir(&self, root: &Path) -> PathBuf {
        match self.subdir {
            Some(sub) => root.join(sub),
            None => root.to_owned(),
        }
    }
}

pub fn find(id: &str) -> Option<&'static ModelSpec> {
    CATALOG.iter().find(|m| m.id == id)
}

pub fn default_spec() -> &'static ModelSpec {
    find(DEFAULT_MODEL_ID).expect("default model must exist in CATALOG")
}

fn asset_ready(dir: &Path, asset: ModelAsset) -> bool {
    fs::metadata(dir.join(asset.file)).is_ok_and(|meta| meta.len() == asset.bytes)
}

fn vad_path(dir: &Path) -> PathBuf {
    dir.join(VAD.file)
}

/// A user-added Whisper GGML model discovered from Hugging Face Hub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomModel {
    pub id: String,
    pub name: String,
    pub description: String,
    pub repo: String,
    pub file: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
}

/// Unified descriptor returned by `ModelStore::list()` for both built-in and custom models.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSummary {
    pub id: String,
    pub name: String,
    pub short_name: String,
    pub description: String,
    pub source: String,
    pub engine: String,
    pub bytes: u64,
    pub ready: bool,
    pub custom: bool,
    /// Operating systems this model runs on: "mac", "windows", "linux".
    pub platforms: Vec<&'static str>,
}

/// Candidate GGML model returned when refreshing/searching Hugging Face Hub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredModel {
    pub id: String,
    pub name: String,
    pub repo: String,
    pub file: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
    pub downloads: u64,
    pub already_added: bool,
}

/// Owns local model storage (built-in catalog + user-added Hugging Face GGML models),
/// single-flight download locking, engine loading, and selection/deletion invariants.
#[derive(Debug, Clone)]
pub struct ModelStore {
    dir: PathBuf,
    downloading: Arc<Mutex<HashSet<String>>>,
}

struct DownloadGuard {
    downloading: Arc<Mutex<HashSet<String>>>,
    model_id: String,
}

impl Drop for DownloadGuard {
    fn drop(&mut self) {
        self.downloading
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.model_id);
    }
}

enum ModelEntry {
    Builtin(&'static ModelSpec),
    Custom(CustomModel),
}

impl ModelEntry {
    fn id(&self) -> &str {
        match self {
            Self::Builtin(s) => s.id,
            Self::Custom(c) => &c.id,
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Builtin(s) => s.name,
            Self::Custom(c) => &c.name,
        }
    }

    fn is_ready(&self, dir: &Path) -> bool {
        if !asset_ready(dir, VAD) {
            return false;
        }
        match self {
            Self::Builtin(s) => {
                let model_dir = s.model_dir(dir);
                s.files.iter().all(|&f| asset_ready(&model_dir, f))
            }
            Self::Custom(c) => {
                fs::metadata(dir.join(&c.file)).is_ok_and(|meta| meta.len() == c.bytes)
            }
        }
    }

    fn to_summary(&self, dir: &Path) -> ModelSummary {
        match self {
            Self::Builtin(m) => ModelSummary {
                id: m.id.to_owned(),
                name: m.name.to_owned(),
                short_name: m.short_name.to_owned(),
                description: m.description.to_owned(),
                source: m.source.to_owned(),
                engine: m.engine.to_owned(),
                bytes: m.total_bytes(),
                ready: self.is_ready(dir),
                custom: false,
                // Qwen3-ASR needs POSIX mmap, so it has no Windows build.
                // Lightweight Whisper GGML models (<= 600 MB) run well on mobile devices.
                platforms: {
                    let mut p = if m.engine_kind == EngineKind::QwenAsr {
                        vec!["mac", "linux"]
                    } else {
                        vec!["mac", "windows", "linux"]
                    };
                    if m.engine_kind == EngineKind::WhisperCpp && m.total_bytes() <= 600_000_000 {
                        p.push("mobile");
                    }
                    p
                },
            },
            Self::Custom(c) => ModelSummary {
                id: c.id.clone(),
                name: c.name.clone(),
                short_name: c.name.clone(),
                description: c.description.clone(),
                source: c.repo.clone(),
                engine: "whisper.cpp".into(),
                bytes: c.bytes,
                ready: self.is_ready(dir),
                custom: true,
                platforms: if c.bytes <= 600_000_000 {
                    vec!["mac", "windows", "linux", "mobile"]
                } else {
                    vec!["mac", "windows", "linux"]
                },
            },
        }
    }
}

impl ModelStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            downloading: Arc::default(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn custom_path(&self) -> PathBuf {
        self.dir.join("custom-models.json")
    }

    fn load_custom(&self) -> Vec<CustomModel> {
        fs::read(self.custom_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save_custom(&self, list: &[CustomModel]) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?;
        save_atomic(&self.custom_path(), &bytes)
            .map_err(|e| format!("không lưu được danh sách model mở rộng: {e}"))
    }

    fn all_entries(&self) -> Vec<ModelEntry> {
        let mut entries: Vec<ModelEntry> = CATALOG.iter().map(ModelEntry::Builtin).collect();
        entries.extend(self.load_custom().into_iter().map(ModelEntry::Custom));
        entries
    }

    fn resolve(&self, model_id: &str) -> Option<ModelEntry> {
        if let Some(spec) = find(model_id) {
            return Some(ModelEntry::Builtin(spec));
        }
        self.load_custom()
            .into_iter()
            .find(|m| m.id == model_id)
            .map(ModelEntry::Custom)
    }

    pub fn has_model(&self, model_id: &str) -> bool {
        self.resolve(model_id).is_some()
    }

    pub fn short_labels(&self) -> BTreeMap<String, String> {
        self.all_entries()
            .into_iter()
            .map(|e| match e {
                ModelEntry::Builtin(m) => (m.id.to_owned(), m.short_name.to_owned()),
                ModelEntry::Custom(c) => (c.id, c.name),
            })
            .collect()
    }

    pub fn list(&self) -> Vec<ModelSummary> {
        self.all_entries()
            .into_iter()
            .map(|e| e.to_summary(&self.dir))
            .collect()
    }

    /// Registers a Whisper GGML model discovered from Hugging Face Hub into the local catalog.
    pub fn add_custom_model(&self, model: CustomModel) -> Result<(), String> {
        if model.id.trim().is_empty()
            || !model.file.ends_with(".bin")
            || model.file.contains('/')
            || model.file.contains("..")
            || !model.url.starts_with("https://huggingface.co/")
            || model.sha256.len() != 64
            || model.bytes == 0
        {
            return Err(
                "Thông tin model Hugging Face không hợp lệ (chỉ hỗ trợ file GGML .bin có SHA-256)."
                    .into(),
            );
        }
        if find(&model.id).is_some() {
            return Err("Model này đã có sẵn trong danh mục mặc định.".into());
        }
        let mut list = self.load_custom();
        if list.iter().any(|m| m.id == model.id) {
            return Ok(());
        }
        list.push(model);
        self.save_custom(&list)
    }

    pub fn is_ready(&self, model_id: &str) -> bool {
        self.resolve(model_id)
            .is_some_and(|e| e.is_ready(&self.dir))
    }

    pub fn are_ready(&self) -> bool {
        self.all_entries().iter().any(|e| e.is_ready(&self.dir))
    }

    /// Ensures `model_id` exists in the catalog (built-in or custom) and has all verified files on disk.
    pub fn validate_select(&self, model_id: &str) -> Result<(), String> {
        let entry = self
            .resolve(model_id)
            .ok_or_else(|| format!("model chép lời không tồn tại: {model_id}"))?;
        if !entry.is_ready(&self.dir) {
            return Err("Model này chưa được tải đầy đủ trên máy.".into());
        }
        Ok(())
    }

    /// Loads `model_id` with the requested resource profile and optional glossary prompt.
    pub fn load(
        &self,
        model_id: &str,
        profile: ResourceProfile,
        initial_prompt: Option<String>,
    ) -> Result<Box<dyn Transcriber>, TranscribeError> {
        let entry = self.resolve(model_id).ok_or_else(|| {
            TranscribeError::Engine(format!("model chép lời không tồn tại: {model_id}"))
        })?;
        if !entry.is_ready(&self.dir) {
            return Err(TranscribeError::Engine(format!(
                "chưa tải đủ model {} để nhận dạng giọng nói, hãy tải trong Cấu hình",
                entry.name()
            )));
        }
        let threads = profile.threads();
        match entry {
            ModelEntry::Builtin(spec) if spec.engine_kind == EngineKind::QwenAsr => {
                let model_dir = spec.model_dir(&self.dir);
                let engine =
                    QwenAsr::load(&model_dir, &vad_path(&self.dir), threads, initial_prompt)?;
                Ok(Box::new(engine))
            }
            ModelEntry::Builtin(spec) => {
                let model_path = spec.model_dir(&self.dir).join(spec.files[0].file);
                let options = WhisperOptions {
                    n_threads: threads as i32,
                    initial_prompt,
                    ..Default::default()
                };
                Ok(Box::new(WhisperCpp::load(
                    &model_path,
                    &vad_path(&self.dir),
                    options,
                )?))
            }
            ModelEntry::Custom(custom) => {
                let options = WhisperOptions {
                    n_threads: threads as i32,
                    initial_prompt,
                    ..Default::default()
                };
                Ok(Box::new(WhisperCpp::load(
                    &self.dir.join(&custom.file),
                    &vad_path(&self.dir),
                    options,
                )?))
            }
        }
    }

    /// Downloads `model_id` (plus Silero VAD if missing) with single-flight locking and SHA-256 verification.
    pub fn download(&self, model_id: &str, on_percent: &dyn Fn(u8)) -> Result<(), String> {
        let entry = self
            .resolve(model_id)
            .ok_or_else(|| format!("model chép lời không tồn tại: {model_id}"))?;
        let _guard = self.acquire_download(entry.id())?;
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;

        let (model_dir, assets): (PathBuf, Vec<(String, String, String, u64)>) = match &entry {
            ModelEntry::Builtin(spec) => {
                let dir = spec.model_dir(&self.dir);
                fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                (
                    dir,
                    spec.files
                        .iter()
                        .map(|f| (f.file.to_owned(), f.url.to_owned(), f.sha256.to_owned(), f.bytes))
                        .collect(),
                )
            }
            ModelEntry::Custom(c) => (
                self.dir.clone(),
                vec![(c.file.clone(), c.url.clone(), c.sha256.clone(), c.bytes)],
            ),
        };

        let total: u64 = assets.iter().map(|a| a.3).sum::<u64>() + VAD.bytes;
        let mut finished = 0u64;
        for (file, url, sha, bytes) in assets {
            fetch_file(&file, &url, &sha, bytes, &model_dir, &|got| {
                on_percent(((finished + got) * 100 / total) as u8)
            })?;
            finished += bytes;
        }
        fetch_file(VAD.file, VAD.url, VAD.sha256, VAD.bytes, &self.dir, &|got| {
            on_percent(((finished + got) * 100 / total) as u8)
        })?;
        on_percent(100);
        Ok(())
    }

    /// Deletes downloaded files for `model_id`, refusing to remove the currently active model.
    pub fn delete(&self, model_id: &str, active_model_id: &str) -> Result<(), String> {
        if active_model_id == model_id {
            return Err("Hãy chọn model khác trước khi xoá model đang dùng.".into());
        }
        let entry = self
            .resolve(model_id)
            .ok_or_else(|| format!("model chép lời không tồn tại: {model_id}"))?;
        match entry {
            ModelEntry::Builtin(spec) => {
                if let Some(sub) = spec.subdir {
                    let sub_path = self.dir.join(sub);
                    if sub_path.exists() {
                        fs::remove_dir_all(&sub_path)
                            .map_err(|e| format!("không xoá được thư mục model: {e}"))?;
                    }
                    return Ok(());
                }
                for asset in spec.files {
                    remove_model_file(&self.dir, asset.file)?;
                }
                Ok(())
            }
            ModelEntry::Custom(custom) => {
                remove_model_file(&self.dir, &custom.file)?;
                let mut list = self.load_custom();
                list.retain(|m| m.id != custom.id);
                self.save_custom(&list)
            }
        }
    }

    /// Searches Hugging Face Hub for Whisper GGML (`.bin`) speech-to-text models and returns
    /// verified LFS entries ready to add & download.
    pub fn discover_hf(&self, query: &str) -> Result<Vec<DiscoveredModel>, String> {
        let existing: HashSet<String> = self.list().into_iter().map(|m| m.id).collect();
        let existing_files: HashSet<String> = CATALOG
            .iter()
            .flat_map(|m| m.files.iter().map(|f| f.file.to_owned()))
            .chain(self.load_custom().into_iter().map(|c| c.file))
            .collect();

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("asr-studio/0.1")
            .build()
            .map_err(|e| e.to_string())?;

        let q = query.trim();
        let search_term = if q.is_empty() { "whisper ggml" } else { q };
        let encoded: String = search_term
            .bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
                b' ' => "+".to_owned(),
                _ => format!("%{b:02X}"),
            })
            .collect();
        let search_url = format!(
            "https://huggingface.co/api/models?search={encoded}&sort=downloads&direction=-1&limit=8"
        );
        let repos: Vec<serde_json::Value> = client
            .get(&search_url)
            .send()
            .map_err(|e| format!("không kết nối được Hugging Face: {}", e.without_url()))?
            .error_for_status()
            .map_err(|e| format!("Hugging Face trả về lỗi: {e}"))?
            .json()
            .map_err(|e| e.to_string())?;

        let mut results = Vec::new();
        for repo_val in repos {
            let Some(repo_id) = repo_val["id"].as_str() else {
                continue;
            };
            let downloads = repo_val["downloads"].as_u64().unwrap_or(0);
            let tree_url = format!("https://huggingface.co/api/models/{repo_id}/tree/main");
            let Ok(resp) = client.get(&tree_url).send() else {
                continue;
            };
            let Ok(files) = resp.json::<Vec<serde_json::Value>>() else {
                continue;
            };
            for file in files {
                let Some(path) = file["path"].as_str() else {
                    continue;
                };
                if !path.ends_with(".bin") || path.contains('/') || path.contains("encoder") || path.contains("vad") {
                    continue;
                }
                let lfs = &file["lfs"];
                let Some(sha256) = lfs["oid"].as_str().filter(|s| s.len() == 64) else {
                    continue;
                };
                let bytes = lfs["size"]
                    .as_u64()
                    .or_else(|| file["size"].as_u64())
                    .unwrap_or(0);
                if bytes < 20_000_000 {
                    continue;
                }
                let stem = path.trim_end_matches(".bin");
                let slug = format!(
                    "hf-{}-{}",
                    repo_id.replace('/', "-").to_ascii_lowercase(),
                    stem.to_ascii_lowercase()
                );
                let already_added = existing.contains(&slug) || existing_files.contains(path);
                results.push(DiscoveredModel {
                    id: slug,
                    name: format!("{stem} ({repo_id})"),
                    repo: repo_id.to_owned(),
                    file: path.to_owned(),
                    url: format!("https://huggingface.co/{repo_id}/resolve/main/{path}"),
                    sha256: sha256.to_owned(),
                    bytes,
                    downloads,
                    already_added,
                });
                if results.len() >= 20 {
                    return Ok(results);
                }
            }
        }
        Ok(results)
    }

    fn acquire_download(&self, model_id: &str) -> Result<DownloadGuard, String> {
        let mut active = self.downloading.lock().unwrap_or_else(|e| e.into_inner());
        if !active.insert(model_id.to_owned()) {
            return Err("Model này đang được tải rồi.".into());
        }
        Ok(DownloadGuard {
            downloading: Arc::clone(&self.downloading),
            model_id: model_id.to_owned(),
        })
    }
}

fn remove_model_file(dir: &Path, file: &str) -> Result<(), String> {
    for path in [dir.join(file), dir.join(format!("{file}.part"))] {
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("không xoá được file model: {e}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
fn fetch(asset: ModelAsset, url: &str, dir: &Path, on_bytes: &dyn Fn(u64)) -> Result<(), String> {
    fetch_file(asset.file, url, asset.sha256, asset.bytes, dir, on_bytes)
}

fn fetch_file(
    file: &str,
    url: &str,
    expected_sha256: &str,
    expected_bytes: u64,
    dir: &Path,
    on_bytes: &dyn Fn(u64),
) -> Result<(), String> {
    let target = dir.join(file);
    if fs::metadata(&target).is_ok_and(|m| m.len() == expected_bytes) {
        on_bytes(expected_bytes);
        return Ok(());
    }
    let part = dir.join(format!("{file}.part"));
    let mut have = fs::metadata(&part).map_or(0, |m| m.len());
    if have > expected_bytes {
        fs::remove_file(&part).map_err(|e| e.to_string())?;
        have = 0;
    }
    if have < expected_bytes {
        let client = reqwest::blocking::Client::builder()
            .timeout(None)
            .build()
            .map_err(|e| e.to_string())?;
        let mut response = client
            .get(url)
            .header("Range", format!("bytes={have}-"))
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("không tải được model: {e}"))?;
        // A server that ignores Range sends the whole file again.
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            have = 0;
        }
        let mut out = OpenOptions::new()
            .create(true)
            .write(true)
            .append(have > 0)
            .truncate(have == 0)
            .open(&part)
            .map_err(|e| e.to_string())?;
        let mut buf = vec![0; 1 << 20];
        loop {
            let n = response
                .read(&mut buf)
                .map_err(|e| format!("mất kết nối khi tải model: {e}"))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            have += n as u64;
            on_bytes(have);
        }
    }
    let actual = sha256(&part).map_err(|e| e.to_string())?;
    if actual != expected_sha256 {
        fs::remove_file(&part).ok();
        return Err(format!("model tải về bị lỗi ({file}), hãy thử lại"));
    }
    fs::rename(&part, &target).map_err(|e| e.to_string())
}

fn sha256(path: &Path) -> io::Result<String> {
    let mut hasher = Sha256::new();
    io::copy(&mut File::open(path)?, &mut hasher)?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_a_known_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc");
        fs::write(&path, "abc").unwrap();
        assert_eq!(
            sha256(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::thread;

    const CONTENT: &[u8] = b"model bytes, small for the test";

    fn asset() -> ModelAsset {
        ModelAsset {
            file: "m.bin",
            url: "unused",
            sha256: Box::leak(sha256_of(CONTENT).into_boxed_str()),
            bytes: CONTENT.len() as u64,
        }
    }

    fn sha256_of(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Serves `body` once. Honours "Range: bytes=N-" with a 206 unless `ignore_range`.
    /// Returns the URL and what Range header, if any, was asked for.
    fn serve(body: &'static [u8], ignore_range: bool) -> (String, thread::JoinHandle<Option<u64>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/m.bin", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut range = None;
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
                if let Some(v) = line.to_lowercase().strip_prefix("range: bytes=") {
                    range = v.trim().trim_end_matches('-').parse::<u64>().ok();
                }
            }
            let (status, part) = match range {
                Some(from) if !ignore_range => ("206 Partial Content", &body[from as usize..]),
                _ => ("200 OK", body),
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                part.len()
            )
            .unwrap();
            stream.write_all(part).unwrap();
            range
        });
        (url, handle)
    }

    #[test]
    fn an_interrupted_download_resumes_where_it_stopped() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("m.bin.part"), &CONTENT[..10]).unwrap();
        let (url, server) = serve(CONTENT, false);
        fetch(asset(), &url, dir.path(), &|_| {}).unwrap();
        assert_eq!(server.join().unwrap(), Some(10));
        assert_eq!(fs::read(dir.path().join("m.bin")).unwrap(), CONTENT);
        assert!(!dir.path().join("m.bin.part").exists());
    }

    #[test]
    fn a_server_that_ignores_range_restarts_the_file_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("m.bin.part"), &CONTENT[..10]).unwrap();
        let (url, server) = serve(CONTENT, true);
        fetch(asset(), &url, dir.path(), &|_| {}).unwrap();
        server.join().unwrap();
        assert_eq!(fs::read(dir.path().join("m.bin")).unwrap(), CONTENT);
    }

    #[test]
    fn a_download_with_the_wrong_hash_is_thrown_away_without_touching_other_models() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("existing.bin"), b"keep me").unwrap();
        let (url, server) = serve(b"other bytes of the same length!", false);
        assert_eq!(b"other bytes of the same length!".len(), CONTENT.len());
        let error = fetch(asset(), &url, dir.path(), &|_| {}).unwrap_err();
        server.join().unwrap();
        assert!(error.contains("bị lỗi"), "{error}");
        assert!(!dir.path().join("m.bin").exists());
        assert!(!dir.path().join("m.bin.part").exists(), "next try starts fresh");
        assert_eq!(fs::read(dir.path().join("existing.bin")).unwrap(), b"keep me");
    }

    #[test]
    fn the_engine_will_not_load_without_the_models() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path().to_path_buf());
        let Err(error) = store.load(DEFAULT_MODEL_ID, ResourceProfile::default(), None) else {
            panic!("loaded from an empty folder")
        };
        assert!(error.to_string().contains("chưa tải đủ model"));
    }

    #[test]
    fn loading_an_unknown_model_reports_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path().to_path_buf());
        let Err(error) = store.load("no-such-model", ResourceProfile::default(), None) else {
            panic!("loaded an unknown model")
        };
        assert!(error.to_string().contains("không tồn tại"));
    }

    #[test]
    fn store_enforces_select_delete_and_single_flight_invariants() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path().to_path_buf());

        assert!(store.validate_select(DEFAULT_MODEL_ID).is_err());
        assert!(store.validate_select("unknown-model").is_err());
        assert!(store.delete(DEFAULT_MODEL_ID, DEFAULT_MODEL_ID).is_err());

        let _guard = store.acquire_download(DEFAULT_MODEL_ID).unwrap();
        assert!(store.acquire_download(DEFAULT_MODEL_ID).is_err());
        drop(_guard);
        assert!(store.acquire_download(DEFAULT_MODEL_ID).is_ok());

        // Custom model registration, listing, and deletion (works for any valid ID, no prefix coupling).
        let custom = CustomModel {
            id: "community-vi-ggml".into(),
            name: "Community VI".into(),
            description: "Test custom model".into(),
            repo: "example/vi-ggml".into(),
            file: "ggml-vi-test.bin".into(),
            url: "https://huggingface.co/example/vi-ggml/resolve/main/ggml-vi-test.bin".into(),
            sha256: "a".repeat(64),
            bytes: 1234,
        };
        store.add_custom_model(custom).unwrap();
        assert!(store.has_model("community-vi-ggml"));
        assert!(store.list().iter().any(|m| m.id == "community-vi-ggml" && m.custom));
        store.delete("community-vi-ggml", DEFAULT_MODEL_ID).unwrap();
        assert!(!store.has_model("community-vi-ggml"));
    }

    #[test]
    fn a_wrong_size_model_is_not_ready() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path().to_path_buf());
        assert!(!store.are_ready());
        fs::write(dir.path().join(default_spec().files[0].file), "short").unwrap();
        assert!(!store.are_ready());
    }

    #[test]
    fn catalog_has_unique_ids_and_valid_default() {
        assert_eq!(default_spec().id, DEFAULT_MODEL_ID);
        assert!(find("whisper-large-v3").is_some());
        assert!(find("qwen3-asr-1.7b").is_some());
        assert!(find("phowhisper-medium-q5_0").is_some());
        let mut ids: Vec<_> = CATALOG.iter().map(|m| m.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), CATALOG.len());
    }
}
