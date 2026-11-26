//! Exports confirmed meeting-note rows to `.xlsx`: either a clean minimal BA-Open-Kit
//! workbook generated on the fly (when no custom template is enabled) or a user-imported `.xlsx`
//! template.
//!
//! Every exported cell is written as an OOXML inline string (`t="inlineStr"`) so Excel
//! always treats it as literal text—even if a note starts with `=` or contains multiple lines.

use std::collections::BTreeMap;
use std::io::{Cursor, Write};
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::Event;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::deliverable::{
    MeetingRow, escape_xml, patch_zip_entry, read_zip_string, resolve_column_value,
};
use crate::output::save_atomic;

/// Extracts column header names from the first table header row in an `.xlsx` template.
pub fn inspect_headers(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("file mẫu không phải định dạng .xlsx hợp lệ: {e}"))?;
    let sheet_name = find_first_sheet(&mut archive)
        .ok_or_else(|| "file mẫu .xlsx không có sheet dữ liệu nào".to_owned())?;
    let shared_strings = read_zip_string(&mut archive, "xl/sharedStrings.xml")
        .map(|xml| parse_shared_strings(&xml))
        .unwrap_or_default();
    let sheet_xml = read_zip_string(&mut archive, &sheet_name)
        .ok_or_else(|| "không đọc được sheet chính của file mẫu".to_owned())?;
    let parsed_rows = parse_sheet_rows(&sheet_xml, &shared_strings);
    let (_, cols) = detect_header_columns(&parsed_rows);
    Ok(cols.into_iter().map(|(_, label)| label).collect())
}

/// Verifies that `bytes` is a valid `.xlsx` archive containing at least one worksheet.
pub fn validate_template(bytes: &[u8]) -> Result<(), String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("file mẫu không phải định dạng .xlsx hợp lệ: {e}"))?;
    find_first_sheet(&mut archive)
        .ok_or_else(|| "file mẫu .xlsx không có sheet dữ liệu nào".to_owned())?;
    Ok(())
}

/// Exports `rows` into `out_path`, patching `template` when provided or generating a clean
/// minimal workbook when `None` (default template disabled).
pub fn export(template: Option<&[u8]>, rows: &[MeetingRow], out_path: &Path) -> Result<(), String> {
    let bytes = match template {
        Some(tpl) => render_xlsx(tpl, rows)?,
        None => render_plain_xlsx(rows)?,
    };
    save_atomic(out_path, &bytes).map_err(|e| format!("không ghi được file Excel: {e}"))
}

fn render_plain_xlsx(rows: &[MeetingRow]) -> Result<Vec<u8>, String> {
    let mut sheet_rows = String::from(
        r#"<row r="1"><c r="A1" t="inlineStr"><is><t>#</t></is></c><c r="B1" t="inlineStr"><is><t>Hạng mục</t></is></c><c r="C1" t="inlineStr"><is><t>Nội dung thống nhất</t></is></c><c r="D1" t="inlineStr"><is><t>Ghi chú</t></is></c></row>"#,
    );
    for (i, row) in rows.iter().enumerate() {
        let row_num = 2 + i as u32;
        let ht = estimate_row_height(row);
        sheet_rows.push_str(&format!(
            r#"<row r="{row_num}" ht="{ht:.2}" customHeight="1">"#
        ));
        sheet_rows.push_str(&inline_str_cell("", 1, row_num, None, &(i + 1).to_string()));
        sheet_rows.push_str(&inline_str_cell("", 2, row_num, None, &row.title));
        sheet_rows.push_str(&inline_str_cell("", 3, row_num, None, &row.agreed));
        sheet_rows.push_str(&inline_str_cell("", 4, row_num, None, &row.note));
        sheet_rows.push_str("</row>");
    }
    let sheet_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cols><col min="1" max="1" width="6" customWidth="1"/><col min="2" max="2" width="28" customWidth="1"/><col min="3" max="3" width="52" customWidth="1"/><col min="4" max="4" width="30" customWidth="1"/></cols><sheetData>{sheet_rows}</sheetData></worksheet>"#
    );
    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#;
    let root_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let workbook = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Biên bản" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;

    let mut out = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut out);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, content) in [
            ("[Content_Types].xml", content_types.as_bytes()),
            ("_rels/.rels", root_rels.as_bytes()),
            ("xl/workbook.xml", workbook.as_bytes()),
            ("xl/_rels/workbook.xml.rels", wb_rels.as_bytes()),
            ("xl/worksheets/sheet1.xml", sheet_xml.as_bytes()),
        ] {
            writer.start_file(name, opts).map_err(|e| e.to_string())?;
            writer.write_all(content).map_err(|e| e.to_string())?;
        }
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(out.into_inner())
}

fn find_first_sheet(archive: &mut ZipArchive<Cursor<&[u8]>>) -> Option<String> {
    let preferred = "xl/worksheets/sheet1.xml";
    if archive.by_name(preferred).is_ok() {
        return Some(preferred.to_owned());
    }
    let mut sheets: Vec<String> = (0..archive.len())
        .filter_map(|i| {
            let name = archive.by_index(i).ok()?.name().to_owned();
            (name.starts_with("xl/worksheets/") && name.ends_with(".xml")).then_some(name)
        })
        .collect();
    sheets.sort();
    sheets.into_iter().next()
}

fn render_xlsx(template_bytes: &[u8], rows: &[MeetingRow]) -> Result<Vec<u8>, String> {
    let mut archive = ZipArchive::new(Cursor::new(template_bytes))
        .map_err(|e| format!("không đọc được file mẫu .xlsx: {e}"))?;
    let sheet_name = find_first_sheet(&mut archive)
        .ok_or_else(|| "file mẫu .xlsx không có sheet dữ liệu".to_owned())?;

    let shared_strings = read_zip_string(&mut archive, "xl/sharedStrings.xml")
        .map(|xml| parse_shared_strings(&xml))
        .unwrap_or_default();
    let sheet_xml = read_zip_string(&mut archive, &sheet_name)
        .ok_or_else(|| "không đọc được sheet chính của file mẫu".to_owned())?;
    let updated_sheet = patch_sheet_xml(&sheet_xml, &shared_strings, rows)?;

    patch_zip_entry(&mut archive, &sheet_name, updated_sheet.as_bytes())
}

fn local_name(raw: &str) -> &str {
    raw.rsplit(':').next().unwrap_or(raw)
}

fn parse_shared_strings(xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    let mut strings = Vec::new();
    let mut in_si = false;
    let mut in_t = false;
    let mut current = String::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match local_name(e.name().as_ref()) {
                "si" => {
                    in_si = true;
                    current.clear();
                }
                "t" if in_si => in_t = true,
                _ => {}
            },
            Ok(Event::Text(e)) if in_si && in_t => {
                current.push_str(e.as_ref());
            }
            Ok(Event::End(e)) => match local_name(e.name().as_ref()) {
                "t" => in_t = false,
                "si" => {
                    in_si = false;
                    strings.push(std::mem::take(&mut current));
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    strings
}

#[derive(Debug, Clone)]
struct ParsedCell {
    col: u32,
    style: Option<String>,
    text: String,
}

#[derive(Debug, Clone)]
struct ParsedRow {
    row_num: u32,
    cells: Vec<ParsedCell>,
}

fn col_from_ref(cell_ref: &str) -> Option<u32> {
    let mut col = 0u32;
    let mut saw_letter = false;
    for b in cell_ref.bytes() {
        if b.is_ascii_alphabetic() {
            saw_letter = true;
            col = col * 26 + u32::from(b.to_ascii_uppercase() - b'A' + 1);
        } else {
            break;
        }
    }
    saw_letter.then_some(col)
}

fn col_to_letters(mut col: u32) -> String {
    let mut out = Vec::new();
    while col > 0 {
        let rem = ((col - 1) % 26) as u8;
        out.push((b'A' + rem) as char);
        col = (col - 1) / 26;
    }
    out.into_iter().rev().collect()
}

fn parse_sheet_rows(sheet_xml: &str, shared_strings: &[String]) -> Vec<ParsedRow> {
    let mut reader = Reader::from_str(sheet_xml);
    let mut rows = Vec::new();
    let mut current_row: Option<ParsedRow> = None;
    let mut current_col: Option<u32> = None;
    let mut current_style: Option<String> = None;
    let mut current_type = String::new();
    let mut current_text = String::new();
    let mut in_v = false;
    let mut in_t = false;
    let mut buf = Vec::new();

    let flush_cell = |row: &mut Option<ParsedRow>,
                      col: &mut Option<u32>,
                      style: &mut Option<String>,
                      cell_type: &mut String,
                      text: &mut String| {
        if let (Some(r), Some(c)) = (row.as_mut(), col.take()) {
            let resolved = if cell_type == "s" {
                text.trim()
                    .parse::<usize>()
                    .ok()
                    .and_then(|idx| shared_strings.get(idx).cloned())
                    .unwrap_or_default()
            } else {
                std::mem::take(text)
            };
            r.cells.push(ParsedCell {
                col: c,
                style: style.take(),
                text: resolved,
            });
        }
        cell_type.clear();
        text.clear();
    };

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match local_name(e.name().as_ref()) {
                "row" => {
                    let row_num = e
                        .attributes()
                        .flatten()
                        .find(|a| local_name(a.key.as_ref()) == "r")
                        .and_then(|a| a.value.parse::<u32>().ok())
                        .unwrap_or(rows.len() as u32 + 1);
                    current_row = Some(ParsedRow {
                        row_num,
                        cells: Vec::new(),
                    });
                }
                "c" if current_row.is_some() => {
                    current_col = None;
                    current_style = None;
                    current_type.clear();
                    current_text.clear();
                    for attr in e.attributes().flatten() {
                        let key = local_name(attr.key.as_ref());
                        let val = attr.value.as_ref();
                        match key {
                            "r" => current_col = col_from_ref(val),
                            "s" => current_style = Some(val.to_owned()),
                            "t" => current_type = val.to_owned(),
                            _ => {}
                        }
                    }
                }
                "v" if current_col.is_some() => in_v = true,
                "t" if current_col.is_some() => in_t = true,
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == "c" && current_row.is_some() {
                    let mut col = None;
                    let mut style = None;
                    for attr in e.attributes().flatten() {
                        let key = local_name(attr.key.as_ref());
                        let val = attr.value.as_ref();
                        match key {
                            "r" => col = col_from_ref(val),
                            "s" => style = Some(val.to_owned()),
                            _ => {}
                        }
                    }
                    if let (Some(r), Some(c)) = (current_row.as_mut(), col) {
                        r.cells.push(ParsedCell {
                            col: c,
                            style,
                            text: String::new(),
                        });
                    }
                }
            }
            Ok(Event::Text(e)) if in_v || in_t => {
                current_text.push_str(e.as_ref());
            }
            Ok(Event::End(e)) => match local_name(e.name().as_ref()) {
                "v" => in_v = false,
                "t" => in_t = false,
                "c" => {
                    flush_cell(
                        &mut current_row,
                        &mut current_col,
                        &mut current_style,
                        &mut current_type,
                        &mut current_text,
                    );
                }
                "row" => {
                    if let Some(r) = current_row.take() {
                        rows.push(r);
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    rows
}

/// Detects the table header row and returns `(header_row_num, Vec<(col_index, header_label)>)`.
/// Works with ANY template structure: first looks for a row with >= 2 recognized meeting-column
/// headers; if none match (arbitrary custom template), picks the first row with >= 2 non-empty
/// text cells so any column layout is preserved verbatim.
fn detect_header_columns(rows: &[ParsedRow]) -> (u32, Vec<(u32, String)>) {
    let is_known_header = |label: &str| {
        let l = label.trim().to_lowercase();
        l == "#"
            || l == "stt"
            || l.contains("số thứ tự")
            || l.contains("hạng mục")
            || l.contains("chủ đề")
            || l.contains("topic")
            || l.contains("thống nhất")
            || l.contains("quyết định")
            || l.contains("nội dung")
            || l.contains("ghi chú")
            || l.contains("phụ trách")
            || l.contains("thời hạn")
            || l.contains("deadline")
            || l.contains("owner")
            || l.contains("pic")
    };

    let extract_cols = |r: &ParsedRow| -> Vec<(u32, String)> {
        let mut cols: Vec<(u32, String)> = r
            .cells
            .iter()
            .filter(|c| !c.text.trim().is_empty())
            .map(|c| (c.col, c.text.trim().to_owned()))
            .collect();
        cols.sort_by_key(|(c, _)| *c);
        cols
    };

    for r in rows {
        let cols = extract_cols(r);
        let hits = cols.iter().filter(|(_, l)| is_known_header(l)).count();
        if hits >= 2 {
            return (r.row_num, cols);
        }
    }
    for r in rows {
        let cols = extract_cols(r);
        if cols.len() >= 2 {
            return (r.row_num, cols);
        }
    }
    (
        1,
        vec![
            (1, "#".into()),
            (2, "Hạng mục".into()),
            (3, "Nội dung thống nhất".into()),
            (4, "Ghi chú".into()),
        ],
    )
}

fn row_styles(rows: &[ParsedRow], row_num: u32) -> BTreeMap<u32, String> {
    rows.iter()
        .find(|r| r.row_num == row_num)
        .map(|r| {
            r.cells
                .iter()
                .filter_map(|c| c.style.clone().map(|s| (c.col, s)))
                .collect()
        })
        .unwrap_or_default()
}

fn inline_str_cell(prefix: &str, col: u32, row_num: u32, style: Option<&str>, text: &str) -> String {
    let cell_ref = format!("{}{}", col_to_letters(col), row_num);
    let style_attr = style.map_or(String::new(), |s| format!(r#" s="{s}""#));
    let escaped = escape_xml(text);
    format!(
        r#"<{prefix}c r="{cell_ref}"{style_attr} t="inlineStr"><{prefix}is><{prefix}t xml:space="preserve">{escaped}</{prefix}t></{prefix}is></{prefix}c>"#
    )
}

fn estimate_row_height(row: &MeetingRow) -> f64 {
    let lines = |s: &str, width: usize| -> usize {
        s.split('\n')
            .map(|part| part.chars().count().div_ceil(width).max(1))
            .sum::<usize>()
            .max(1)
    };
    let max_lines = lines(&row.title, 30)
        .max(lines(&row.agreed, 44))
        .max(lines(&row.note, 26));
    (max_lines as f64 * 16.5 + 9.0).max(25.5)
}

fn patch_sheet_xml(
    sheet_xml: &str,
    shared_strings: &[String],
    rows: &[MeetingRow],
) -> Result<String, String> {
    let parsed_rows = parse_sheet_rows(sheet_xml, shared_strings);
    let (header_row_num, cols) = detect_header_columns(&parsed_rows);
    let odd_styles = row_styles(&parsed_rows, header_row_num + 1);
    let even_styles = {
        let s = row_styles(&parsed_rows, header_row_num + 2);
        if s.is_empty() { odd_styles.clone() } else { s }
    };

    // Detect namespace prefix used by <sheetData> (e.g. "x:" or "").
    let (open_tag, close_tag, prefix) = if sheet_xml.contains("<x:sheetData") {
        ("<x:sheetData>", "</x:sheetData>", "x:")
    } else if sheet_xml.contains("<sheetData") {
        ("<sheetData>", "</sheetData>", "")
    } else {
        return Err("sheet trong file mẫu không có thẻ sheetData".to_owned());
    };

    let open_pos = sheet_xml
        .find(open_tag)
        .ok_or_else(|| "không tìm thấy thẻ mở sheetData".to_owned())?;
    let body_start = open_pos + open_tag.len();
    let close_pos = sheet_xml[body_start..]
        .find(close_tag)
        .map(|p| body_start + p)
        .ok_or_else(|| "không tìm thấy thẻ đóng sheetData".to_owned())?;

    let existing_body = &sheet_xml[body_start..close_pos];
    let kept_prefix = keep_rows_up_to(existing_body, prefix, header_row_num);

    let mut generated = String::new();
    for (i, row) in rows.iter().enumerate() {
        let row_num = header_row_num + 1 + i as u32;
        let styles = if i % 2 == 0 { &odd_styles } else { &even_styles };
        let ht = estimate_row_height(row);
        generated.push_str(&format!(
            r#"<{prefix}row r="{row_num}" ht="{ht:.2}" customHeight="1">"#
        ));

        for (col, header) in &cols {
            let text = resolve_column_value(header, i, row);
            let style = styles.get(col).map(String::as_str);
            generated.push_str(&inline_str_cell(prefix, *col, row_num, style, &text));
        }
        generated.push_str(&format!("</{prefix}row>"));
    }

    let mut result = String::with_capacity(sheet_xml.len() + generated.len());
    result.push_str(&sheet_xml[..body_start]);
    result.push_str(&kept_prefix);
    result.push_str(&generated);
    result.push_str(&sheet_xml[close_pos..]);
    Ok(result)
}

/// Keeps raw `<row ...>...</row>` XML up to and including `max_row_num`.
fn keep_rows_up_to(sheet_data_body: &str, prefix: &str, max_row_num: u32) -> String {
    let row_open = format!("<{prefix}row");
    let row_close = format!("</{prefix}row>");
    let mut kept = String::new();
    let mut cursor = 0;
    while let Some(rel_start) = sheet_data_body[cursor..].find(&row_open) {
        let start = cursor + rel_start;
        let Some(tag_end_rel) = sheet_data_body[start..].find('>') else {
            break;
        };
        let tag_end = start + tag_end_rel;
        let open_slice = &sheet_data_body[start..=tag_end];
        let self_closing = open_slice.ends_with("/>");
        let end = if self_closing {
            tag_end + 1
        } else {
            let Some(close_rel) = sheet_data_body[tag_end + 1..].find(&row_close) else {
                break;
            };
            tag_end + 1 + close_rel + row_close.len()
        };
        let row_num = extract_row_num(open_slice).unwrap_or(0);
        if row_num > 0 && row_num <= max_row_num {
            kept.push_str(&sheet_data_body[start..end]);
        }
        cursor = end;
    }
    kept
}

fn extract_row_num(open_tag: &str) -> Option<u32> {
    let pos = open_tag.find(r#"r=""#)?;
    let rest = &open_tag[pos + 3..];
    let end = rest.find('"')?;
    rest[..end].parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_default_template_with_confirmed_rows_and_text_cells() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("bien-ban.xlsx");
        let rows = vec![
            MeetingRow {
                title: "Đổi format ID".into(),
                agreed: "Giữ nguyên mã hiện tại".into(),
                note: "Anh Nam xác nhận".into(),
                fields: BTreeMap::new(),
            },
            MeetingRow {
                title: "Chốt lịch release".into(),
                agreed: "Release thứ Sáu\nDeploy lúc 20:00".into(),
                note: "Chị Hạnh phụ trách".into(),
                fields: BTreeMap::new(),
            },
        ];
        export(None, &rows, &out).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let xml = read_zip_string(&mut archive, "xl/worksheets/sheet1.xml").unwrap();
        assert!(xml.contains("Hạng mục"));
        assert!(xml.contains("Đổi format ID"));
        assert!(xml.contains("Chị Hạnh phụ trách"));
        // Only header + 2 exported rows.
        assert_eq!(xml.matches("<row ").count(), 3);
    }

    #[test]
    fn formula_like_multiline_notes_stay_plain_text() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("formula.xlsx");
        let rows = vec![MeetingRow {
            title: "Kiểm tra công thức".into(),
            agreed: "Thống nhất giữ nguyên".into(),
            note: "=SUM(A1:A5)\nDòng thứ hai".into(),
            fields: BTreeMap::new(),
        }];
        export(None, &rows, &out).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let xml = read_zip_string(&mut archive, "xl/worksheets/sheet1.xml").unwrap();
        assert!(xml.contains(r#"t="inlineStr""#));
        assert!(xml.contains("=SUM(A1:A5)\nDòng thứ hai"));
        assert!(!xml.contains("<x:f>"), "exported cells must not contain formulas");
    }

    #[test]
    fn custom_template_preserves_banner_rows_and_maps_columns_by_header() {
        // Build a minimal custom xlsx with a banner on row 1 and headers on row 2.
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut z = ZipWriter::new(&mut cursor);
            let opts = SimpleFileOptions::default();
            z.start_file("xl/worksheets/sheet1.xml", opts).unwrap();
            z.write_all(
                r#"<?xml version="1.0" encoding="utf-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>BIEN BAN CONG TY</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>STT</t></is></c><c r="B2" t="inlineStr"><is><t>Hạng mục</t></is></c><c r="C2" t="inlineStr"><is><t>Nội dung thống nhất</t></is></c><c r="D2" t="inlineStr"><is><t>Ghi chú</t></is></c></row><row r="3"><c r="A3" s="5" /><c r="B3" s="6" /><c r="C3" s="7" /><c r="D3" s="8" /></row></sheetData></worksheet>"#.as_bytes(),
            )
            .unwrap();
            z.finish().unwrap();
        }
        let custom = cursor.into_inner();
        validate_template(&custom).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("custom.xlsx");
        export(
            Some(&custom),
            &[MeetingRow {
                title: "Mục A".into(),
                agreed: "Đã chốt".into(),
                note: "Ghi chú A".into(),
                fields: BTreeMap::new(),
            }],
            &out,
        )
        .unwrap();
        let bytes = std::fs::read(&out).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let xml = read_zip_string(&mut archive, "xl/worksheets/sheet1.xml").unwrap();
        assert!(xml.contains("BIEN BAN CONG TY"), "banner row preserved");
        assert!(xml.contains(r#"r="A3" s="5""#), "template cell style preserved");
        assert!(xml.contains("Mục A"));
        assert!(xml.contains("Ghi chú A"));
    }

    #[test]
    fn invalid_template_is_rejected() {
        assert!(validate_template(b"not a zip").is_err());
    }
}
