//! Shared OOXML deliverable primitives for Excel (`.xlsx`) and Word (`.docx`) meeting exports.
//!
//! Centralizes the row model, column-value resolution, XML escaping, ZIP entry patching, and
//! format dispatch so `excel` and `docx` never reach across each other's seams.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::{docx, excel};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TemplateFormat {
    Excel,
    Docx,
}

impl TemplateFormat {
    pub fn from_path(path: &Path) -> Result<Self, String> {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("xlsx") => Ok(Self::Excel),
            Some("docx") => Ok(Self::Docx),
            _ => Err("Chỉ hỗ trợ file mẫu Word (.docx) hoặc Excel (.xlsx).".into()),
        }
    }

    pub fn storage_filename(self) -> &'static str {
        match self {
            Self::Excel => "meeting-template.xlsx",
            Self::Docx => "meeting-template.docx",
        }
    }

    pub fn meta_filename(self) -> &'static str {
        match self {
            Self::Excel => "meeting-template.json",
            Self::Docx => "meeting-template-docx.json",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeetingRow {
    pub title: String,
    pub agreed: String,
    pub note: String,
    /// Dynamic column values keyed by arbitrary template header names.
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
}

pub fn validate_template(format: TemplateFormat, bytes: &[u8]) -> Result<(), String> {
    match format {
        TemplateFormat::Excel => excel::validate_template(bytes),
        TemplateFormat::Docx => docx::validate_template(bytes),
    }
}

pub fn inspect_headers(format: TemplateFormat, bytes: &[u8]) -> Result<Vec<String>, String> {
    match format {
        TemplateFormat::Excel => excel::inspect_headers(bytes),
        TemplateFormat::Docx => docx::inspect_headers(bytes),
    }
}

/// Escapes text for XML 1.0 text nodes and strips illegal control characters.
pub(crate) fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\r' => {}
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

/// Maps a template column header to the corresponding cell text for `row`.
pub(crate) fn resolve_column_value(header: &str, row_index: usize, row: &MeetingRow) -> String {
    let h = header.trim();
    if let Some(v) = row
        .fields
        .iter()
        .find(|(k, _)| k.trim().eq_ignore_ascii_case(h))
        .map(|(_, v)| v.trim())
        && !v.is_empty()
    {
        return v.to_owned();
    }
    let l = h.to_lowercase();
    if l == "#" || l == "stt" || l.contains("số thứ tự") || l == "no" || l == "no." {
        return (row_index + 1).to_string();
    }
    if l.contains("hạng mục")
        || l.contains("chủ đề")
        || l.contains("topic")
        || l.contains("vấn đề")
        || l.contains("đầu mục")
        || l == "mục"
    {
        return row.title.clone();
    }
    if l.contains("thống nhất")
        || l.contains("quyết định")
        || l.contains("kết luận")
        || l.contains("nội dung")
        || l.contains("agreed")
        || l.contains("decision")
    {
        return row.agreed.clone();
    }
    if l.contains("ghi chú") || l.contains("note") || l.contains("remark") {
        return row.note.clone();
    }
    String::new()
}

/// Reads a UTF-8 text entry from an OOXML ZIP archive.
pub(crate) fn read_zip_string(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<String> {
    let mut file = archive.by_name(name).ok()?;
    let mut s = String::new();
    file.read_to_string(&mut s).ok()?;
    Some(s)
}

/// Copies an OOXML ZIP archive while replacing the single `target_entry` with `replacement`.
pub(crate) fn patch_zip_entry(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    target_entry: &str,
    replacement: &[u8],
) -> Result<Vec<u8>, String> {
    let mut out = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut out);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
            let name = file.name().to_owned();
            if file.is_dir() {
                writer.add_directory(&name, opts).map_err(|e| e.to_string())?;
                continue;
            }
            writer.start_file(&name, opts).map_err(|e| e.to_string())?;
            if name == target_entry {
                writer.write_all(replacement).map_err(|e| e.to_string())?;
            } else {
                let mut buf = Vec::new();
                file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
                writer.write_all(&buf).map_err(|e| e.to_string())?;
            }
        }
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(out.into_inner())
}
