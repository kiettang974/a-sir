//! The user's list of correct terms, used only when writing a summary.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

const CSV_HEADER: [&str; 3] = ["Viết đúng", "Hay bị nghe thành", "Giải thích"];
const BOM: &str = "\u{feff}";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Term {
    pub correct: String,
    #[serde(default)]
    pub heard_as: String,
    #[serde(default)]
    pub note: String,
}

pub fn load(path: &Path) -> Vec<Term> {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, terms: &[Term]) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(&tidy(terms)).map_err(|e| e.to_string())?;
    fs::write(path, bytes).map_err(|e| e.to_string())
}

/// Drops rows with no correct term, which say nothing.
fn tidy(terms: &[Term]) -> Vec<Term> {
    terms
        .iter()
        .map(|t| Term {
            correct: t.correct.trim().to_owned(),
            heard_as: t.heard_as.trim().to_owned(),
            note: t.note.trim().to_owned(),
        })
        .filter(|t| !t.correct.is_empty())
        .collect()
}

/// With a BOM so Excel opens Vietnamese correctly.
pub fn to_csv(terms: &[Term]) -> Result<String, String> {
    let mut writer = csv::Writer::from_writer(Vec::new());
    let fail = |e: csv::Error| e.to_string();
    writer.write_record(CSV_HEADER).map_err(fail)?;
    for t in terms {
        writer
            .write_record([&t.correct, &t.heard_as, &t.note])
            .map_err(fail)?;
    }
    let bytes = writer.into_inner().map_err(|e| e.to_string())?;
    Ok(format!("{BOM}{}", String::from_utf8_lossy(&bytes)))
}

/// Accepts the first row as a header if it matches ours. Excel in a Vietnamese locale
/// saves with semicolons, so that delimiter is recognised too.
pub fn from_csv(text: &str) -> Result<Vec<Term>, String> {
    let text = text.strip_prefix(BOM).unwrap_or(text);
    let first = text.lines().next().unwrap_or("");
    let delimiter = if first.contains(';') && !first.contains(',') {
        b';'
    } else {
        b','
    };
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(text.as_bytes());
    let mut terms = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| format!("file CSV lỗi: {e}"))?;
        let cell = |i: usize| record.get(i).unwrap_or("").to_owned();
        if cell(0).trim() == CSV_HEADER[0] {
            continue;
        }
        terms.push(Term {
            correct: cell(0),
            heard_as: cell(1),
            note: cell(2),
        });
    }
    Ok(tidy(&terms))
}

/// Imported rows replace existing ones with the same correct term; the rest are kept.
pub fn merge(existing: &[Term], imported: &[Term]) -> Vec<Term> {
    let by_term: HashMap<&str, &Term> = imported.iter().map(|t| (t.correct.as_str(), t)).collect();
    let mut merged: Vec<Term> = existing
        .iter()
        .map(|t| {
            by_term
                .get(t.correct.as_str())
                .map_or_else(|| t.clone(), |n| (*n).clone())
        })
        .collect();
    for term in imported {
        if !existing.iter().any(|t| t.correct == term.correct)
            && !merged.iter().any(|t| t.correct == term.correct)
        {
            merged.push(term.clone());
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(correct: &str, heard_as: &str, note: &str) -> Term {
        Term {
            correct: correct.into(),
            heard_as: heard_as.into(),
            note: note.into(),
        }
    }

    #[test]
    fn csv_round_trips_vietnamese_commas_and_quotes() {
        let terms = vec![term(
            "LCN",
            "Long Châu, el xi en",
            "Hệ thống \"LabConnect\"",
        )];
        let text = to_csv(&terms).unwrap();
        assert!(text.starts_with(BOM));
        assert_eq!(from_csv(&text).unwrap(), terms);
    }

    #[test]
    fn excel_semicolons_short_rows_and_blank_terms_are_handled() {
        let text = "Viết đúng;Hay bị nghe thành;Giải thích\nBravo;ba vô\n;không có từ;x\n";
        assert_eq!(from_csv(text).unwrap(), vec![term("Bravo", "ba vô", "")]);
    }

    #[test]
    fn a_file_without_our_header_keeps_its_first_row() {
        assert_eq!(
            from_csv("ERP,e rờ pê,\n").unwrap(),
            vec![term("ERP", "e rờ pê", "")]
        );
    }

    #[test]
    fn import_updates_matching_terms_and_adds_new_ones() {
        let existing = vec![term("ERP", "cũ", ""), term("LCN", "giữ", "")];
        let imported = vec![term("ERP", "mới", ""), term("Bravo", "", "")];
        assert_eq!(
            merge(&existing, &imported),
            vec![
                term("ERP", "mới", ""),
                term("LCN", "giữ", ""),
                term("Bravo", "", "")
            ]
        );
    }

    #[test]
    fn save_and_load_round_trip_and_tidy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("g.json");
        assert!(load(&path).is_empty());
        save(&path, &[term(" LCN ", "", ""), term("  ", "x", "")]).unwrap();
        assert_eq!(load(&path), vec![term("LCN", "", "")]);
    }
}
