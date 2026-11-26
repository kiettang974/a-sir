//! Exports confirmed meeting-note rows to `.docx` (Word) using either a clean built-in
//! layout or a user-imported `.docx` template (preserving headers, logos, and table styles).

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

/// Extracts table column headers from the first `<w:tbl>` in a `.docx` template, if any.
pub fn inspect_headers(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("file mẫu không phải định dạng .docx hợp lệ: {e}"))?;
    let doc_xml = read_zip_string(&mut archive, "word/document.xml")
        .ok_or_else(|| "file mẫu .docx không có word/document.xml".to_owned())?;
    Ok(extract_first_table_headers(&doc_xml))
}

fn extract_first_table_headers(doc_xml: &str) -> Vec<String> {
    let Some(tbl_start) = doc_xml.find("<w:tbl") else {
        return Vec::new();
    };
    let Some(tbl_end_rel) = doc_xml[tbl_start..].find("</w:tbl>") else {
        return Vec::new();
    };
    let tbl = &doc_xml[tbl_start..tbl_start + tbl_end_rel + "</w:tbl>".len()];
    let Some(tr_start) = tbl.find("<w:tr") else {
        return Vec::new();
    };
    let Some(tr_end_rel) = tbl[tr_start..].find("</w:tr>") else {
        return Vec::new();
    };
    let tr = &tbl[tr_start..tr_start + tr_end_rel + "</w:tr>".len()];

    let mut reader = Reader::from_str(tr);
    let mut headers = Vec::new();
    let mut in_tc = false;
    let mut in_t = false;
    let mut current = String::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                "w:tc" => {
                    in_tc = true;
                    current.clear();
                }
                "w:t" if in_tc => in_t = true,
                _ => {}
            },
            Ok(Event::Text(e)) if in_tc && in_t => current.push_str(e.as_ref()),
            Ok(Event::End(e)) => match e.name().as_ref() {
                "w:t" => in_t = false,
                "w:tc" => {
                    in_tc = false;
                    headers.push(std::mem::take(&mut current).trim().to_owned());
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    headers
}

/// Verifies that `bytes` is a valid `.docx` archive containing `word/document.xml`.
pub fn validate_template(bytes: &[u8]) -> Result<(), String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("file mẫu không phải định dạng .docx hợp lệ: {e}"))?;
    archive
        .by_name("word/document.xml")
        .map_err(|_| "file mẫu .docx không có word/document.xml".to_owned())?;
    Ok(())
}

/// Exports `rows` to a `.docx` file at `out_path`, patching `template` when provided
/// or generating a styled built-in Word document when `None`.
pub fn export(
    template: Option<&[u8]>,
    title: &str,
    greeting: &str,
    intro: &str,
    rows: &[MeetingRow],
    out_path: &Path,
) -> Result<(), String> {
    let bytes = match template {
        Some(tpl) => render_from_template(tpl, rows)?,
        None => render_default_docx(title, greeting, intro, rows)?,
    };
    save_atomic(out_path, &bytes).map_err(|e| format!("không ghi được file Word: {e}"))
}

fn text_runs(text: &str, bold: bool, color: Option<&str>) -> String {
    let mut rpr = String::from(
        r#"<w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:cs="Times New Roman"/><w:sz w:val="22"/><w:szCs w:val="22"/>"#,
    );
    if bold {
        rpr.push_str("<w:b/>");
    }
    if let Some(c) = color {
        rpr.push_str(&format!(r#"<w:color w:val="{c}"/>"#));
    }
    rpr.push_str("</w:rPr>");

    let mut out = String::new();
    for (idx, line) in text.split('\n').enumerate() {
        if idx > 0 {
            out.push_str("<w:r><w:br/></w:r>");
        }
        out.push_str(&format!(
            r#"<w:r>{rpr}<w:t xml:space="preserve">{}</w:t></w:r>"#,
            escape_xml(line)
        ));
    }
    out
}

fn default_cell(text: &str, width_dxa: u32, fill: Option<&str>, bold: bool, color: Option<&str>) -> String {
    let shd = fill.map_or(String::new(), |f| {
        format!(r#"<w:shd w:val="clear" w:color="auto" w:fill="{f}"/>"#)
    });
    format!(
        r#"<w:tc><w:tcPr><w:tcW w:w="{width_dxa}" w:type="dxa"/>{shd}<w:vAlign w:val="center"/></w:tcPr><w:p>{}</w:p></w:tc>"#,
        text_runs(text, bold, color)
    )
}

fn build_default_table(rows: &[MeetingRow]) -> String {
    let mut tbl = String::from(
        r#"<w:tbl><w:tblPr><w:tblW w:w="9360" w:type="dxa"/><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="D9D9D9"/><w:left w:val="single" w:sz="4" w:space="0" w:color="D9D9D9"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="D9D9D9"/><w:right w:val="single" w:sz="4" w:space="0" w:color="D9D9D9"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="D9D9D9"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="D9D9D9"/></w:tblBorders><w:tblCellMar><w:top w:w="80" w:type="dxa"/><w:left w:w="120" w:type="dxa"/><w:bottom w:w="80" w:type="dxa"/><w:right w:w="120" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="660"/><w:gridCol w:w="2300"/><w:gridCol w:w="4100"/><w:gridCol w:w="2300"/></w:tblGrid>"#,
    );
    tbl.push_str("<w:tr><w:trPr><w:tblHeader/></w:trPr>");
    tbl.push_str(&default_cell("#", 660, Some("2F5D46"), true, Some("FFFFFF")));
    tbl.push_str(&default_cell("Hạng mục", 2300, Some("2F5D46"), true, Some("FFFFFF")));
    tbl.push_str(&default_cell("Nội dung thống nhất", 4100, Some("2F5D46"), true, Some("FFFFFF")));
    tbl.push_str(&default_cell("Ghi chú", 2300, Some("2F5D46"), true, Some("FFFFFF")));
    tbl.push_str("</w:tr>");

    for (i, row) in rows.iter().enumerate() {
        let fill = if i % 2 == 1 { Some("F3F5F7") } else { Some("FFFFFF") };
        tbl.push_str("<w:tr>");
        tbl.push_str(&default_cell(&(i + 1).to_string(), 660, fill, false, None));
        tbl.push_str(&default_cell(&row.title, 2300, fill, false, None));
        tbl.push_str(&default_cell(&row.agreed, 4100, fill, false, None));
        tbl.push_str(&default_cell(&row.note, 2300, fill, false, None));
        tbl.push_str("</w:tr>");
    }
    tbl.push_str("</w:tbl>");
    tbl
}

fn render_default_docx(
    title: &str,
    greeting: &str,
    intro: &str,
    rows: &[MeetingRow],
) -> Result<Vec<u8>, String> {
    let mut body = String::new();
    if !title.trim().is_empty() {
        body.push_str(&format!("<w:p>{}</w:p>", text_runs(title.trim(), true, None)));
    }
    if !greeting.trim().is_empty() {
        body.push_str(&format!("<w:p>{}</w:p>", text_runs(greeting.trim(), false, None)));
    }
    if !intro.trim().is_empty() {
        body.push_str(&format!("<w:p>{}</w:p>", text_runs(intro.trim(), false, None)));
    }
    body.push_str(&build_default_table(rows));
    body.push_str(
        r#"<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1134" w:right="1134" w:bottom="1134" w:left="1134"/></w:sectPr>"#,
    );

    let document_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

    let mut out = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut out);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, content) in [
            ("[Content_Types].xml", content_types.as_bytes()),
            ("_rels/.rels", rels.as_bytes()),
            ("word/document.xml", document_xml.as_bytes()),
        ] {
            writer.start_file(name, opts).map_err(|e| e.to_string())?;
            writer.write_all(content).map_err(|e| e.to_string())?;
        }
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(out.into_inner())
}

fn render_from_template(template_bytes: &[u8], rows: &[MeetingRow]) -> Result<Vec<u8>, String> {
    let mut archive = ZipArchive::new(Cursor::new(template_bytes))
        .map_err(|e| format!("không đọc được file mẫu .docx: {e}"))?;
    let doc_xml = read_zip_string(&mut archive, "word/document.xml")
        .ok_or_else(|| "file mẫu .docx không có word/document.xml".to_owned())?;
    let updated_doc = patch_document_xml(&doc_xml, rows);
    patch_zip_entry(&mut archive, "word/document.xml", updated_doc.as_bytes())
}

/// If the template already has a `<w:tbl>`, keeps its properties and header row and replaces
/// its body rows with `rows`. Otherwise inserts a formatted table before `</w:body>`.
fn patch_document_xml(doc_xml: &str, rows: &[MeetingRow]) -> String {
    if let (Some(tbl_start), Some(tbl_end_rel)) = (
        doc_xml.find("<w:tbl"),
        doc_xml
            .find("<w:tbl")
            .and_then(|s| doc_xml[s..].find("</w:tbl>")),
    ) {
        let tbl_end = tbl_start + tbl_end_rel + "</w:tbl>".len();
        let tbl = &doc_xml[tbl_start..tbl_end];
        if let (Some(first_tr_start), Some(first_tr_end_rel)) = (
            tbl.find("<w:tr"),
            tbl.find("<w:tr").and_then(|s| tbl[s..].find("</w:tr>")),
        ) {
            let header_end = first_tr_start + first_tr_end_rel + "</w:tr>".len();
            let header_part = &tbl[..header_end];
            let headers = extract_first_table_headers(doc_xml);
            let col_count = headers.len().max(1);
            let mut new_rows = String::new();
            for (i, row) in rows.iter().enumerate() {
                let fill = if i % 2 == 1 { Some("F3F5F7") } else { Some("FFFFFF") };
                let values: Vec<String> = if headers.iter().any(|h| !h.is_empty()) {
                    headers
                        .iter()
                        .map(|h| resolve_column_value(h, i, row))
                        .collect()
                } else if col_count >= 4 {
                    let mut v = vec![
                        (i + 1).to_string(),
                        row.title.clone(),
                        row.agreed.clone(),
                        row.note.clone(),
                    ];
                    v.resize(col_count, String::new());
                    v
                } else if col_count == 3 {
                    vec![row.title.clone(), row.agreed.clone(), row.note.clone()]
                } else if col_count == 2 {
                    vec![row.title.clone(), format!("{}\n{}", row.agreed, row.note).trim().to_owned()]
                } else {
                    vec![format!("{}: {}", row.title, row.agreed)]
                };
                new_rows.push_str("<w:tr>");
                for val in values {
                    let shd = fill.map_or(String::new(), |f| {
                        format!(r#"<w:shd w:val="clear" w:color="auto" w:fill="{f}"/>"#)
                    });
                    new_rows.push_str(&format!(
                        r#"<w:tc><w:tcPr>{shd}</w:tcPr><w:p>{}</w:p></w:tc>"#,
                        text_runs(&val, false, None)
                    ));
                }
                new_rows.push_str("</w:tr>");
            }
            let mut out = String::with_capacity(doc_xml.len() + new_rows.len());
            out.push_str(&doc_xml[..tbl_start]);
            out.push_str(header_part);
            out.push_str(&new_rows);
            out.push_str("</w:tbl>");
            out.push_str(&doc_xml[tbl_end..]);
            return out;
        }
    }

    let table = build_default_table(rows);
    if let Some(pos) = doc_xml.rfind("<w:sectPr") {
        let mut out = String::with_capacity(doc_xml.len() + table.len());
        out.push_str(&doc_xml[..pos]);
        out.push_str(&table);
        out.push_str(&doc_xml[pos..]);
        out
    } else if let Some(pos) = doc_xml.rfind("</w:body>") {
        let mut out = String::with_capacity(doc_xml.len() + table.len());
        out.push_str(&doc_xml[..pos]);
        out.push_str(&table);
        out.push_str(&doc_xml[pos..]);
        out
    } else {
        doc_xml.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    #[test]
    fn exports_default_docx_with_multiline_text_and_valid_zip() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("bien-ban.docx");
        let rows = vec![MeetingRow {
            title: "Chốt lịch".into(),
            agreed: "Release thứ Sáu\nDeploy 20:00".into(),
            note: "Anh Nam phụ trách".into(),
            fields: std::collections::BTreeMap::new(),
        }];
        export(None, "Họp kế toán", "Dear team,", "Gửi biên bản.", &rows, &out).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        validate_template(&bytes).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let mut xml = String::new();
        archive.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
        assert!(xml.contains("Họp kế toán"));
        assert!(xml.contains("Chốt lịch"));
        assert!(xml.contains("<w:br/>"));
        assert!(xml.contains("Anh Nam phụ trách"));
    }

    #[test]
    fn custom_docx_template_preserves_header_and_fills_table() {
        let dir = tempfile::tempdir().unwrap();
        let tpl_path = dir.path().join("tpl.docx");
        export(None, "BIEN BAN CONG TY", "", "", &[], &tpl_path).unwrap();
        let tpl_bytes = std::fs::read(&tpl_path).unwrap();

        let out = dir.path().join("filled.docx");
        export(
            Some(&tpl_bytes),
            "ignored",
            "",
            "",
            &[MeetingRow {
                title: "Mục 1".into(),
                agreed: "Đã thống nhất".into(),
                note: "Ghi chú 1".into(),
                fields: std::collections::BTreeMap::new(),
            }],
            &out,
        )
        .unwrap();
        let bytes = std::fs::read(&out).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let mut xml = String::new();
        archive.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
        assert!(xml.contains("BIEN BAN CONG TY"), "template heading preserved");
        assert!(xml.contains("Mục 1"));
        assert!(xml.contains("Ghi chú 1"));
    }

    #[test]
    fn invalid_docx_template_is_rejected() {
        assert!(validate_template(b"not a docx").is_err());
    }
}
