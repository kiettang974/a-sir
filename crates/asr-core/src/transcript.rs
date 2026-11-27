//! A finished transcript and the files that sit beside it: the cleaned copy and the
//! meeting-note draft. Callers get one by job id from `JobRunner::transcript` and never
//! build these paths themselves.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::clean;
use crate::glossary::Term;
use crate::output::{candidates, save_atomic};

/// One transcript line. `start` is "HH:MM:SS", or empty for a line without a timestamp.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Line {
    pub start: String,
    pub text: String,
}

/// Reads one TXT line, "[HH:MM:SS --> HH:MM:SS] text", into its times and its text.
/// The only parser of the format `output::to_txt` writes.
pub fn split_line(line: &str) -> (Option<(&str, &str)>, &str) {
    let parsed = line
        .strip_prefix('[')
        .and_then(|rest| rest.split_once("] "))
        .and_then(|(times, text)| {
            let (start, end) = times.split_once(" --> ")?;
            Some(((start, end), text))
        });
    match parsed {
        Some((times, text)) => (Some(times), text),
        None => (None, line),
    }
}

/// The non-blank lines of a TXT transcript.
pub fn lines(text: &str) -> Vec<Line> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let (times, text) = split_line(line);
            Line {
                start: times.map_or_else(String::new, |(s, _)| s.to_owned()),
                text: text.to_owned(),
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct Transcript {
    txt: PathBuf,
    source: PathBuf,
}

impl Transcript {
    pub(crate) fn new(txt: PathBuf, source: PathBuf) -> Self {
        Self { txt, source }
    }

    /// The recording it was made from.
    pub fn source(&self) -> &Path {
        &self.source
    }

    pub fn text(&self) -> Result<String, String> {
        fs::read_to_string(&self.txt)
            .map_err(|e| format!("không đọc được {}: {e}", self.txt.display()))
    }

    /// Returns the cleaned transcript (`_ai_fixed.txt`, then `_fixed.txt`) when present,
    /// falling back to the raw transcript.
    pub fn text_for_summary(&self) -> Result<String, String> {
        for suffix in ["_ai_fixed", "_fixed"] {
            let candidate = self.sidecar(suffix, "txt");
            if let Ok(content) = fs::read_to_string(&candidate)
                && !content.trim().is_empty()
            {
                return Ok(content);
            }
        }
        self.text()
    }

    pub fn lines(&self) -> Result<Vec<Line>, String> {
        Ok(lines(&self.text()?))
    }

    fn sidecar(&self, suffix: &str, extension: &str) -> PathBuf {
        let stem = self.txt.file_stem().unwrap_or_default().to_string_lossy();
        self.txt
            .with_file_name(format!("{stem}{suffix}.{extension}"))
    }

    /// Writes "<name>_fixed.txt" beside the transcript and returns its path. Like every
    /// transcript file it never overwrites: an identical earlier copy is reused, and a
    /// different one (perhaps edited by hand) is kept and the new copy becomes "_fixed (2)".
    pub fn write_cleaned(&self) -> Result<PathBuf, String> {
        self.write_cleaned_with_glossary(&[])
    }

    /// Writes "<name>_fixed.txt" applying both built-in cleanup and user glossary replacements.
    pub fn write_cleaned_with_glossary(&self, terms: &[Term]) -> Result<PathBuf, String> {
        let cleaned = clean::clean_transcript_with_glossary(&self.text()?, terms);
        self.write_variant("_fixed", &cleaned)
    }

    /// Writes (or overwrites on re-run) the AI-cleaned copy as "<name>_ai_fixed.txt"
    /// and optional diff metadata as "<name>_ai_fixed.json" without touching the original transcript.
    pub fn write_ai_cleaned(&self, cleaned: &str) -> Result<PathBuf, String> {
        let path = self.sidecar("_ai_fixed", "txt");
        save_atomic(&path, cleaned.as_bytes())
            .map_err(|e| format!("không ghi được {}: {e}", path.display()))?;
        Ok(path)
    }

    pub fn save_ai_clean_meta(&self, meta: &Value) -> Result<(), String> {
        save_atomic(&self.sidecar("_ai_fixed", "json"), meta.to_string().as_bytes())
            .map_err(|e| e.to_string())
    }

    pub fn load_ai_clean_meta(&self) -> Option<Value> {
        let bytes = fs::read(self.sidecar("_ai_fixed", "json")).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn write_variant(&self, suffix: &str, content: &str) -> Result<PathBuf, String> {
        let dir = self.txt.parent().unwrap_or(Path::new("."));
        let stem = format!(
            "{}{suffix}",
            self.txt.file_stem().unwrap_or_default().to_string_lossy()
        );
        for mut paths in candidates(dir, &stem, &["txt"]) {
            let path = paths.remove(0);
            match fs::read_to_string(&path) {
                Ok(existing) if existing == content => return Ok(path),
                Ok(_) => continue,
                Err(_) if path.exists() => continue,
                Err(_) => {
                    save_atomic(&path, content.as_bytes())
                        .map_err(|e| format!("không ghi được {}: {e}", path.display()))?;
                    return Ok(path);
                }
            }
        }
        unreachable!("candidates never run out")
    }

    /// The saved meeting-note draft, if there is one. Its shape belongs to the page.
    pub fn note(&self) -> Result<Option<Value>, String> {
        match fs::read(self.sidecar("_note", "json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| format!("bản nháp bị hỏng: {e}")),
            Err(_) => Ok(None),
        }
    }

    /// Saves the working draft and the finished page beside the transcript and returns the
    /// page's path. These two are the app's own working files, so each save replaces the last.
    pub fn save_note(&self, draft: &Value, html: &str) -> Result<PathBuf, String> {
        let page = self.sidecar("_note", "html");
        save_atomic(&self.sidecar("_note", "json"), draft.to_string().as_bytes())
            .map_err(|e| e.to_string())?;
        save_atomic(&page, html.as_bytes()).map_err(|e| e.to_string())?;
        Ok(page)
    }

    /// Path of the latest saved process diagram for this recording.
    pub fn diagram_path(&self) -> PathBuf {
        self.sidecar("_diagram", "json")
    }

    /// Loads the latest saved process diagram for this recording, if present.
    pub fn diagram(&self) -> Result<Option<Value>, String> {
        let path = self.diagram_path();
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| format!("tệp sơ đồ đã lưu bị hỏng: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("không đọc được sơ đồ đã lưu: {e}")),
        }
    }

    /// Atomically saves the latest process diagram beside the transcript so a failed write
    /// never corrupts the previously saved diagram.
    pub fn save_diagram(&self, diagram: &Value) -> Result<PathBuf, String> {
        let path = self.diagram_path();
        let bytes = serde_json::to_vec_pretty(diagram).map_err(|e| e.to_string())?;
        save_atomic(&path, &bytes).map_err(|e| format!("không lưu được sơ đồ: {e}"))?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::output::to_txt;
    use crate::transcriber::Segment;

    fn transcript(dir: &Path, text: &str) -> Transcript {
        let txt = dir.join("họp.txt");
        fs::write(&txt, text).unwrap();
        Transcript::new(txt, dir.join("họp.m4a"))
    }

    #[test]
    fn lines_read_back_what_to_txt_writes() {
        let text = to_txt(&[Segment {
            start_ms: 3_661_000,
            end_ms: 3_670_000,
            text: "xin chào".into(),
        }]);
        assert_eq!(
            lines(&format!("{text}\n   \nkhông có mốc\n")),
            vec![
                Line {
                    start: "01:01:01".into(),
                    text: "xin chào".into()
                },
                Line {
                    start: "".into(),
                    text: "không có mốc".into()
                },
            ]
        );
    }

    #[test]
    fn a_bracket_that_is_not_a_timestamp_stays_text() {
        assert_eq!(split_line("[ghi chú] nội dung"), (None, "[ghi chú] nội dung"));
    }

    #[test]
    fn cleaning_twice_reuses_the_copy_and_never_overwrites_an_edited_one() {
        let dir = tempfile::tempdir().unwrap();
        let t = transcript(dir.path(), "[00:00:00 --> 00:00:01] tài hiệu\n");
        let first = t.write_cleaned().unwrap();
        assert_eq!(first, dir.path().join("họp_fixed.txt"));
        assert_eq!(t.write_cleaned().unwrap(), first, "same content, same file");

        fs::write(&first, "đã sửa tay").unwrap();
        let second = t.write_cleaned().unwrap();
        assert_eq!(second, dir.path().join("họp_fixed (2).txt"));
        assert_eq!(fs::read_to_string(&first).unwrap(), "đã sửa tay");
        assert_eq!(
            fs::read_to_string(&second).unwrap(),
            "[00:00:00 --> 00:00:01] tài liệu\n"
        );
    }

    #[test]
    fn the_note_round_trips_beside_the_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let t = transcript(dir.path(), "");
        assert_eq!(t.note().unwrap(), None);
        let draft = json!({"rows": [], "pending": []});
        let page = t.save_note(&draft, "<p>bảng</p>").unwrap();
        assert_eq!(page, dir.path().join("họp_note.html"));
        assert_eq!(fs::read_to_string(page).unwrap(), "<p>bảng</p>");
        assert_eq!(t.note().unwrap(), Some(draft));
    }

    #[test]
    fn summary_prefers_ai_cleaned_then_local_cleaned_then_raw() {
        let dir = tempfile::tempdir().unwrap();
        let t = transcript(dir.path(), "[00:00:00 --> 00:00:01] bản gốc\n");
        assert!(t.text_for_summary().unwrap().contains("bản gốc"));

        t.write_cleaned().unwrap();
        fs::write(dir.path().join("họp_fixed.txt"), "[00:00:00 --> 00:00:01] bản lọc máy\n").unwrap();
        assert!(t.text_for_summary().unwrap().contains("bản lọc máy"));

        t.write_ai_cleaned("[00:00:00 --> 00:00:01] bản lọc AI\n").unwrap();
        assert!(t.text_for_summary().unwrap().contains("bản lọc AI"));
    }

    #[test]
    fn diagrams_round_trip_separately_per_transcript_file() {
        let dir = tempfile::tempdir().unwrap();
        let t1 = transcript(dir.path(), "bản 1");
        let txt2 = dir.path().join("họp (2).txt");
        fs::write(&txt2, "bản 2").unwrap();
        let t2 = Transcript::new(txt2, dir.path().join("họp.m4a"));

        assert_eq!(t1.diagram().unwrap(), None);
        assert_eq!(t2.diagram().unwrap(), None);

        let d1 = json!({ "title": "Sơ đồ 1", "nodes": [], "edges": [] });
        let d2 = json!({ "title": "Sơ đồ 2", "nodes": [], "edges": [] });
        let p1 = t1.save_diagram(&d1).unwrap();
        let p2 = t2.save_diagram(&d2).unwrap();
        assert_ne!(p1, p2);
        assert_eq!(t1.diagram().unwrap(), Some(d1));
        assert_eq!(t2.diagram().unwrap(), Some(d2));
    }
}
