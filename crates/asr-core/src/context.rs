//! Per-recording meeting context: free-form notes and copied file attachments stored under
//! the app's private data directory, keyed by `job_id` so recordings with identical titles
//! keep isolated context and survive source file renames or app restarts.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::ai::AiAttachment;
use crate::output::save_atomic;

pub const MAX_FILE_BYTES: u64 = 15 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 40 * 1024 * 1024;
pub const ACCEPTED_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "pdf", "txt", "md"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextAttachment {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub bytes: u64,
    pub stored_file: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingContext {
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub attachments: Vec<ContextAttachment>,
}

impl MeetingContext {
    pub fn is_empty(&self) -> bool {
        self.notes.trim().is_empty() && self.attachments.is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddAttachmentsResult {
    pub context: MeetingContext,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ContextStore {
    root: PathBuf,
}

impl ContextStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            root: data_dir.join("contexts"),
        }
    }

    fn job_dir(&self, job_id: u64) -> PathBuf {
        self.root.join(format!("job-{job_id}"))
    }

    fn meta_path(&self, job_id: u64) -> PathBuf {
        self.job_dir(job_id).join("context.json")
    }

    fn files_dir(&self, job_id: u64) -> PathBuf {
        self.job_dir(job_id).join("files")
    }

    pub fn load(&self, job_id: u64) -> MeetingContext {
        let mut ctx: MeetingContext = fs::read(self.meta_path(job_id))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        // Drop any stale metadata whose stored file disappeared on disk.
        let files_dir = self.files_dir(job_id);
        ctx.attachments
            .retain(|a| files_dir.join(&a.stored_file).is_file());
        ctx
    }

    fn save(&self, job_id: u64, ctx: &MeetingContext) -> Result<(), String> {
        fs::create_dir_all(self.job_dir(job_id)).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(ctx).map_err(|e| e.to_string())?;
        save_atomic(&self.meta_path(job_id), &bytes).map_err(|e| e.to_string())
    }

    pub fn save_notes(&self, job_id: u64, notes: &str) -> Result<MeetingContext, String> {
        let mut ctx = self.load(job_id);
        ctx.notes = notes.to_owned();
        self.save(job_id, &ctx)?;
        Ok(ctx)
    }

    /// Copies valid files into app-private storage, keeping existing data intact if any file fails.
    pub fn add_files(&self, job_id: u64, paths: &[PathBuf]) -> Result<AddAttachmentsResult, String> {
        let mut ctx = self.load(job_id);
        let mut errors = Vec::new();
        let files_dir = self.files_dir(job_id);
        let mut current_total: u64 = ctx.attachments.iter().map(|a| a.bytes).sum();
        let mut changed = false;

        for path in paths {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| path.display().to_string());

            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .unwrap_or_default();

            let Some(mime_type) = mime_for_ext(&ext) else {
                errors.push(format!(
                    "{name}: chưa hỗ trợ định dạng này (chỉ nhận PNG, JPG, PDF, TXT, Markdown)."
                ));
                continue;
            };

            let meta = match fs::metadata(path) {
                Ok(m) if m.is_file() => m,
                Ok(_) => {
                    errors.push(format!("{name}: không phải tệp hợp lệ."));
                    continue;
                }
                Err(e) => {
                    errors.push(format!("{name}: không đọc được tệp ({e})."));
                    continue;
                }
            };

            let size = meta.len();
            if size == 0 {
                errors.push(format!("{name}: tệp rỗng."));
                continue;
            }
            if size > MAX_FILE_BYTES {
                errors.push(format!(
                    "{name}: vượt giới hạn {} MB mỗi tệp.",
                    MAX_FILE_BYTES / (1024 * 1024)
                ));
                continue;
            }
            if current_total.saturating_add(size) > MAX_TOTAL_BYTES {
                errors.push(format!(
                    "{name}: vượt tổng giới hạn {} MB cho một bản ghi.",
                    MAX_TOTAL_BYTES / (1024 * 1024)
                ));
                continue;
            }

            if let Err(e) = fs::create_dir_all(&files_dir) {
                errors.push(format!("{name}: không tạo được thư mục lưu trữ ({e})."));
                continue;
            }

            let id = next_attachment_id(&ctx.attachments);
            let stored_file = format!("{id}.{ext}");
            let dest = files_dir.join(&stored_file);
            if let Err(e) = fs::copy(path, &dest) {
                errors.push(format!("{name}: không sao chép được tệp ({e})."));
                continue;
            }

            current_total = current_total.saturating_add(size);
            ctx.attachments.push(ContextAttachment {
                id,
                name,
                mime_type: mime_type.to_owned(),
                bytes: size,
                stored_file,
            });
            changed = true;
        }

        if changed {
            self.save(job_id, &ctx)?;
        }

        Ok(AddAttachmentsResult {
            context: ctx,
            errors,
        })
    }

    /// Removes an attachment copy from app storage without touching the user's original source file.
    pub fn remove_file(&self, job_id: u64, attachment_id: &str) -> Result<MeetingContext, String> {
        let mut ctx = self.load(job_id);
        let files_dir = self.files_dir(job_id);
        if let Some(pos) = ctx.attachments.iter().position(|a| a.id == attachment_id) {
            let removed = ctx.attachments.remove(pos);
            let _ = fs::remove_file(files_dir.join(&removed.stored_file));
            self.save(job_id, &ctx)?;
        }
        Ok(ctx)
    }

    /// Reads stored attachments into text or base64 payloads for multimodal AI calls.
    pub fn load_ai_attachments(&self, job_id: u64) -> Result<(String, Vec<AiAttachment>), String> {
        let ctx = self.load(job_id);
        let files_dir = self.files_dir(job_id);
        let mut out = Vec::with_capacity(ctx.attachments.len());

        for att in &ctx.attachments {
            let path = files_dir.join(&att.stored_file);
            let bytes = fs::read(&path)
                .map_err(|e| format!("không đọc được tệp đính kèm \"{}\": {e}", att.name))?;
            if att.mime_type.starts_with("text/") {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                out.push(AiAttachment::Text {
                    name: att.name.clone(),
                    content: text,
                });
            } else {
                out.push(AiAttachment::Binary {
                    name: att.name.clone(),
                    mime_type: att.mime_type.clone(),
                    base64: encode_base64(&bytes),
                });
            }
        }

        Ok((ctx.notes, out))
    }
}

fn mime_for_ext(ext: &str) -> Option<&'static str> {
    match ext {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "pdf" => Some("application/pdf"),
        "txt" => Some("text/plain"),
        "md" | "markdown" => Some("text/markdown"),
        _ => None,
    }
}

fn next_attachment_id(existing: &[ContextAttachment]) -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut seq = existing.len() + 1;
    loop {
        let candidate = format!("att-{ts}-{seq}");
        if !existing.iter().any(|a| a.id == candidate) {
            return candidate;
        }
        seq += 1;
    }
}

/// Standard RFC 4648 base64 encoder (no external dependency needed).
pub fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_standard_vectors() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn notes_and_files_survive_source_deletion_and_stay_isolated_per_job() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::new(dir.path());
        store.save_notes(1, "Hệ thống Alpha, phụ trách Lan").unwrap();
        store.save_notes(2, "Cuộc họp khác").unwrap();

        let src = dir.path().join("agenda.md");
        fs::write(&src, "# Agenda\n- Chốt phạm vi").unwrap();
        let added = store.add_files(1, std::slice::from_ref(&src)).unwrap();
        assert!(added.errors.is_empty());
        assert_eq!(added.context.attachments.len(), 1);

        // Removing the original source file must not break the stored copy.
        fs::remove_file(&src).unwrap();

        let store_reloaded = ContextStore::new(dir.path());
        let (notes1, atts1) = store_reloaded.load_ai_attachments(1).unwrap();
        assert_eq!(notes1, "Hệ thống Alpha, phụ trách Lan");
        assert_eq!(atts1.len(), 1);
        assert_eq!(store_reloaded.load(2).notes, "Cuộc họp khác");
        assert!(store_reloaded.load(2).attachments.is_empty());

        let att_id = added.context.attachments[0].id.clone();
        let after_remove = store_reloaded.remove_file(1, &att_id).unwrap();
        assert!(after_remove.attachments.is_empty());
        assert_eq!(after_remove.notes, "Hệ thống Alpha, phụ trách Lan");
    }

    #[test]
    fn invalid_files_report_errors_without_losing_saved_context() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::new(dir.path());
        store.save_notes(10, "Ghi chú cũ").unwrap();

        let bad = dir.path().join("archive.zip");
        fs::write(&bad, b"PK\x03\x04").unwrap();
        let empty = dir.path().join("empty.txt");
        fs::write(&empty, b"").unwrap();
        let good = dir.path().join("diagram.png");
        fs::write(&good, b"\x89PNG\r\n\x1a\nfake").unwrap();

        let res = store.add_files(10, &[bad, empty, good]).unwrap();
        assert_eq!(res.errors.len(), 2);
        assert!(res.errors[0].contains("archive.zip"));
        assert!(res.errors[1].contains("empty.txt"));
        assert_eq!(res.context.notes, "Ghi chú cũ");
        assert_eq!(res.context.attachments.len(), 1);
        assert_eq!(res.context.attachments[0].mime_type, "image/png");
    }
}
