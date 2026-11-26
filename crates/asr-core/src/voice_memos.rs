//! Reads what the macOS Voice Memos app has stored. Its database layout is private and may
//! change, so columns are looked up at runtime and a plain folder scan is the fallback.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::{Connection, OpenFlags, types::Value};
use serde::Serialize;

/// Core Data dates count seconds from 2001-01-01.
const CORE_DATA_EPOCH: i64 = 978_307_200;
const AUDIO_EXTENSIONS: [&str; 8] = ["m4a", "qta", "mp3", "wav", "aac", "flac", "ogg", "wma"];

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    pub path: PathBuf,
    pub title: String,
    /// Unix seconds.
    pub recorded_at: Option<i64>,
    pub duration_s: Option<f64>,
    /// False when the recording is still only in iCloud.
    pub on_disk: bool,
}

#[derive(Debug, PartialEq)]
pub enum ListError {
    /// macOS blocks this folder until the app has Full Disk Access.
    NoPermission,
    Other(String),
}

#[cfg(target_os = "macos")]
pub fn folder(home: &Path) -> PathBuf {
    home.join("Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings")
}

#[cfg(not(target_os = "macos"))]
pub fn folder(home: &Path) -> PathBuf {
    home.join("Documents").join("Sound Recordings")
}

/// Newest first. A folder that doesn't exist simply means no recordings.
pub fn list(dir: &Path) -> Result<Vec<Recording>, ListError> {
    let scanned = match scan_folder(dir) {
        Ok(found) => found,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) if e.kind() == ErrorKind::PermissionDenied => return Err(ListError::NoPermission),
        Err(e) => {
            return Err(ListError::Other(format!(
                "không đọc được thư mục Ghi âm: {e}"
            )));
        }
    };
    let mut recordings = match read_database(dir) {
        Some(from_db) if !from_db.is_empty() => from_db,
        _ => scanned,
    };
    recordings.sort_by_key(|r| std::cmp::Reverse(r.recorded_at));
    Ok(recordings)
}

fn scan_folder(dir: &Path) -> std::io::Result<Vec<Recording>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let is_audio = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_lowercase().as_str()));
        if !is_audio {
            continue;
        }
        let recorded_at = fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);
        found.push(Recording {
            title: stem(&path),
            path,
            recorded_at,
            duration_s: None,
            on_disk: true,
        });
    }
    Ok(found)
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned())
}

/// `None` whenever the database isn't what we expect, so the caller can scan instead.
fn read_database(dir: &Path) -> Option<Vec<Recording>> {
    let db = Connection::open_with_flags(
        dir.join("CloudRecordings.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    let mut info = db.prepare("PRAGMA table_info(ZCLOUDRECORDING)").ok()?;
    let columns: Vec<String> = info
        .query_map([], |row| row.get(1))
        .ok()?
        .collect::<Result<_, _>>()
        .ok()?;
    let has = |name: &str| columns.iter().any(|c| c == name);
    if !has("ZPATH") {
        return None;
    }
    let column = |name: &'static str| if has(name) { name } else { "NULL" };
    let not_deleted = if has("ZEVICTIONDATE") {
        "ZEVICTIONDATE IS NULL"
    } else {
        "1"
    };
    let sql = format!(
        "SELECT ZPATH, {}, {}, {}, {}, {} FROM ZCLOUDRECORDING WHERE {not_deleted}",
        column("ZENCRYPTEDTITLE"),
        column("ZCUSTOMLABEL"),
        column("ZCUSTOMLABELFORSORTING"),
        column("ZDATE"),
        column("ZDURATION"),
    );
    let mut query = db.prepare(&sql).ok()?;
    let rows = query
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Value>(1)?,
                row.get::<_, Value>(2)?,
                row.get::<_, Value>(3)?,
                row.get::<_, Value>(4)?,
                row.get::<_, Value>(5)?,
            ))
        })
        .ok()?;

    let mut recordings = Vec::new();
    for row in rows {
        let (file, encrypted, label, sorting, date, duration) = row.ok()?;
        // Stored as a file name; keeping only the last part means it can't point elsewhere.
        let Some(name) = file.as_deref().and_then(|f| Path::new(f).file_name()) else {
            continue;
        };
        let path = dir.join(name);
        let title = [encrypted, label, sorting]
            .into_iter()
            .find_map(|v| match v {
                Value::Text(t) if !t.trim().is_empty() => Some(t),
                _ => None,
            })
            .unwrap_or_else(|| stem(&path));
        recordings.push(Recording {
            on_disk: path.exists(),
            title,
            recorded_at: number(&date).map(|d| d as i64 + CORE_DATA_EPOCH),
            duration_s: number(&duration),
            path,
        });
    }
    Some(recordings)
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Real(n) => Some(*n),
        Value::Integer(n) => Some(*n as f64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database(dir: &Path, schema: &str, rows: &[&str]) {
        let db = Connection::open(dir.join("CloudRecordings.db")).unwrap();
        db.execute(schema, []).unwrap();
        for row in rows {
            db.execute(row, []).unwrap();
        }
    }

    const FULL: &str = "CREATE TABLE ZCLOUDRECORDING (ZPATH TEXT, ZENCRYPTEDTITLE TEXT, \
                        ZCUSTOMLABEL TEXT, ZDATE REAL, ZDURATION REAL, ZEVICTIONDATE REAL)";

    #[test]
    fn the_database_gives_titles_dates_and_lengths_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.m4a"), "x").unwrap();
        fs::write(dir.path().join("b.m4a"), "x").unwrap();
        database(
            dir.path(),
            FULL,
            &[
                "INSERT INTO ZCLOUDRECORDING VALUES ('a.m4a', 'Cũ', NULL, 100.0, 60.5, NULL)",
                "INSERT INTO ZCLOUDRECORDING VALUES ('b.m4a', NULL, 'Mới', 200.0, 90.0, NULL)",
            ],
        );
        let found = list(dir.path()).unwrap();
        assert_eq!(
            found.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
            ["Mới", "Cũ"]
        );
        assert_eq!(found[1].recorded_at, Some(100 + CORE_DATA_EPOCH));
        assert_eq!(found[1].duration_s, Some(60.5));
        assert!(found[1].on_disk);
    }

    #[test]
    fn recently_deleted_is_hidden_and_icloud_only_is_flagged() {
        let dir = tempfile::tempdir().unwrap();
        database(
            dir.path(),
            FULL,
            &[
                "INSERT INTO ZCLOUDRECORDING VALUES ('gone.m4a', 'Đã xoá', NULL, 1.0, 1.0, 50.0)",
                "INSERT INTO ZCLOUDRECORDING VALUES ('cloud.m4a', 'Trên mây', NULL, 2.0, 1.0, NULL)",
            ],
        );
        let found = list(dir.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Trên mây");
        assert!(!found[0].on_disk);
    }

    #[test]
    fn a_stored_path_cannot_leave_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        database(
            dir.path(),
            FULL,
            &["INSERT INTO ZCLOUDRECORDING VALUES ('/etc/passwd', 'x', NULL, 1.0, 1.0, NULL)"],
        );
        assert_eq!(list(dir.path()).unwrap()[0].path, dir.path().join("passwd"));
    }

    #[test]
    fn a_missing_column_is_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        database(
            dir.path(),
            "CREATE TABLE ZCLOUDRECORDING (ZPATH TEXT, ZDATE REAL)",
            &["INSERT INTO ZCLOUDRECORDING VALUES ('c.m4a', 5.0)"],
        );
        let found = list(dir.path()).unwrap();
        assert_eq!(found[0].title, "c");
        assert_eq!(found[0].duration_s, None);
    }

    #[test]
    fn an_unknown_database_falls_back_to_the_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("20261109 1.m4a"), "x").unwrap();
        fs::write(dir.path().join("20261109 2.QTA"), "x").unwrap();
        fs::write(dir.path().join("ghi chú.txt"), "x").unwrap();
        database(dir.path(), "CREATE TABLE KHAC (A TEXT)", &[]);
        let mut titles: Vec<_> = list(dir.path())
            .unwrap()
            .into_iter()
            .map(|r| r.title)
            .collect();
        titles.sort();
        assert_eq!(titles, ["20261109 1", "20261109 2"]);
    }

    #[test]
    fn a_missing_folder_means_no_recordings() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(list(&dir.path().join("không có")), Ok(Vec::new()));
    }
}
