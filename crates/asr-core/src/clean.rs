//! Rule-based transcript cleanup: known Whisper hallucinations, stutter loops,
//! and optional user glossary replacements (`heard_as` → `correct`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unicode_normalization::UnicodeNormalization;

use crate::ai::{AiAttachment, AiError, LlmClient};
use crate::glossary::Term;
use crate::transcript;

const SPELLING: [(&str, &str); 2] = [("tài hiệu", "tài liệu"), ("Tài hiệu", "Tài liệu")];
const PHANTOM_STARTS: [&str; 6] = [
    "Các bạn hãy đăng ký kênh",
    "Hãy subscribe cho kênh",
    "Các bạn có thể nhớ like",
    "nhấn chuông để không bỏ lỡ",
    "ủng hộ kênh của mình",
    "Cảm ơn các bạn đã theo dõi",
];
const MIN_REPEATS: usize = 5;

/// "à à à à à" → "à [à ×5]". The old pattern needed a regex backreference.
fn collapse_repeats(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let norm = |w: &str| {
        w.trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase()
    };
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let word = norm(words[i]);
        let run = if word.is_empty() {
            1
        } else {
            words[i..].iter().take_while(|w| norm(w) == word).count()
        };
        if run >= MIN_REPEATS {
            out.push(format!("{} [{} ×{run}]", words[i], word));
        } else {
            out.extend(words[i..i + run].iter().map(|w| w.to_string()));
        }
        i += run;
    }
    out.join(" ")
}

/// Builds `(heard_as_lower_nfc, correct)` pairs sorted longest-first so multi-word phrases
/// win over shorter substrings. Supports multiple variants separated by `,`, `;`, `/`, or `|`.
fn glossary_rules(terms: &[Term]) -> Vec<(String, String)> {
    let mut rules: Vec<(String, String)> = Vec::new();
    for term in terms {
        let correct = term.correct.trim().nfc().collect::<String>();
        if correct.is_empty() {
            continue;
        }
        for raw in term.heard_as.split([',', ';', '/', '|']) {
            let needle = raw.trim().nfc().collect::<String>().to_lowercase();
            if !needle.is_empty() && needle != correct.to_lowercase() {
                rules.push((needle, correct.clone()));
            }
        }
    }
    rules.sort_by_key(|a| std::cmp::Reverse(a.0.chars().count()));
    rules
}

/// Case-insensitive, Unicode-NFC word-boundary replacement so short glossary terms never
/// corrupt unrelated words.
fn apply_glossary(text: &str, rules: &[(String, String)]) -> String {
    if rules.is_empty() {
        return text.to_owned();
    }
    let mut current: String = text.nfc().collect();
    for (needle, replacement) in rules {
        let needle_chars: Vec<char> = needle.chars().collect();
        let n_len = needle_chars.len();
        if n_len == 0 {
            continue;
        }
        let orig_chars: Vec<char> = current.chars().collect();
        let lower_chars: Vec<char> = orig_chars
            .iter()
            .map(|c| c.to_lowercase().next().unwrap_or(*c))
            .collect();
        let mut out = String::with_capacity(current.len());
        let mut i = 0;
        while i < orig_chars.len() {
            let fits = i + n_len <= lower_chars.len()
                && lower_chars[i..i + n_len] == needle_chars[..];
            let left_ok = i == 0 || !orig_chars[i - 1].is_alphanumeric();
            let right_ok =
                fits && (i + n_len == orig_chars.len() || !orig_chars[i + n_len].is_alphanumeric());
            if fits && left_ok && right_ok {
                out.push_str(replacement);
                i += n_len;
            } else {
                out.push(orig_chars[i]);
                i += 1;
            }
        }
        current = out;
    }
    current
}

/// Phantom lines are dropped outright; the old script left blank lines behind.
pub fn clean_transcript(txt: &str) -> String {
    clean_transcript_with_glossary(txt, &[])
}

/// Cleans hallucinations, spelling, stutter loops, and replaces glossary `heard_as` phrases
/// with `correct` on word boundaries while preserving timestamps.
pub fn clean_transcript_with_glossary(txt: &str, terms: &[Term]) -> String {
    let rules = glossary_rules(terms);
    txt.lines()
        .filter_map(|line| {
            let (times, text) = transcript::split_line(line);
            let prefix = times.map_or_else(String::new, |(s, e)| format!("[{s} --> {e}] "));
            if PHANTOM_STARTS.iter().any(|p| text.starts_with(p)) {
                return None;
            }
            let fixed = SPELLING
                .iter()
                .fold(text.to_owned(), |t, (from, to)| t.replace(from, to));
            let collapsed = collapse_repeats(&fixed);
            let replaced = apply_glossary(&collapsed, &rules);
            Some(format!("{prefix}{replaced}\n"))
        })
        .collect()
}

/// One proposed AI correction to a single transcript line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCleanEdit {
    pub line: usize,
    pub time: String,
    pub original: String,
    pub suggested: String,
    pub reason: String,
}

/// Preview of AI-assisted cleaning: line-level diff plus full reconstructed transcript text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCleanProposal {
    pub edits: Vec<AiCleanEdit>,
    pub cleaned_text: String,
}

#[derive(Deserialize)]
struct RawCleanReply {
    #[serde(default)]
    edits: Vec<RawCleanEdit>,
}

#[derive(Deserialize)]
struct RawCleanEdit {
    line: usize,
    corrected: String,
    #[serde(default)]
    reason: String,
}

fn ai_clean_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "edits": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "line": { "type": "integer" },
                        "corrected": { "type": "string" },
                        "reason": { "type": "string" }
                    },
                    "required": ["line", "corrected", "reason"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["edits"],
        "additionalProperties": false
    })
}

/// Asks AI to propose grounded terminology/proper-noun fixes while preserving every timestamp,
/// number, negation, and original meaning. Returns a reviewable diff and reconstructed text.
pub fn propose_ai_clean(
    client: &dyn LlmClient,
    transcript_txt: &str,
    terms: &[Term],
    context_notes: &str,
    attachments: &[AiAttachment],
) -> Result<AiCleanProposal, AiError> {
    let raw_lines: Vec<&str> = transcript_txt.lines().collect();
    let mut numbered = String::new();
    let mut parsed_lines: Vec<(String, String, String)> = Vec::with_capacity(raw_lines.len());

    for (idx, raw) in raw_lines.iter().enumerate() {
        let (times, text) = transcript::split_line(raw);
        let prefix = times.map_or_else(String::new, |(s, e)| format!("[{s} --> {e}] "));
        let start = times.map_or_else(String::new, |(s, _)| s.to_owned());
        if !text.trim().is_empty() {
            numbered.push_str(&format!("{}| {}\n", idx + 1, text));
        }
        parsed_lines.push((prefix, start, text.to_owned()));
    }

    let system = "Bạn rà soát bản chép lời tự động tiếng Việt để đề xuất sửa lỗi nghe nhầm tên riêng, tên hệ thống, chữ viết tắt và thuật ngữ chuyên môn.\n\
        Quy tắc bắt buộc:\n\
        - CHỈ sửa khi có căn cứ rõ ràng từ ngữ cảnh câu nói, từ vựng riêng hoặc ghi chú/tài liệu đính kèm.\n\
        - Nếu còn nghi ngờ, giữ nguyên nguyên văn lời nói (không đưa vào `edits`).\n\
        - Ngữ cảnh có thể cung cấp chính tả đúng của một tên, nhưng việc gán tên đó cho người hoặc hệ thống cụ thể phải có căn cứ từ chính đoạn chép.\n\
        - TUYỆT ĐỐI giữ nguyên ý nghĩa, số liệu, điều kiện, phủ định và cấu trúc câu.\n\
        - Không thêm, bớt dòng và không tự viết mốc giờ vào `corrected`.\n\
        - Chỉ trả về các dòng thực sự có thay đổi trong mảng `edits` (mảng rỗng nếu không cần sửa).";

    let mut user = String::new();
    if !terms.is_empty() {
        user.push_str("Từ vựng riêng:\n");
        for t in terms {
            if !t.correct.trim().is_empty() {
                user.push_str(&format!("- {} (hay nghe nhầm: {})\n", t.correct.trim(), t.heard_as.trim()));
            }
        }
        user.push('\n');
    }
    if !context_notes.trim().is_empty() {
        user.push_str(&format!(
            "Ghi chú ngữ cảnh cuộc họp:\n{}\n\n",
            context_notes.trim()
        ));
    }
    user.push_str("Bản chép lời (định dạng `<số dòng>| <nội dung>`):\n");
    user.push_str(&numbered);

    let reply = if attachments.is_empty() {
        client.complete(system, &user, &ai_clean_schema())?
    } else {
        client.complete_with_attachments(system, &user, &ai_clean_schema(), attachments)?
    };
    let trimmed = reply
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let parsed: RawCleanReply = serde_json::from_str(trimmed)
        .map_err(|e| AiError::Malformed(format!("kết quả làm sạch không đúng định dạng: {e}")))?;

    let mut by_line: BTreeMap<usize, (String, String)> = BTreeMap::new();
    for item in parsed.edits {
        if item.line == 0 || item.line > parsed_lines.len() {
            continue;
        }
        let corrected = item.corrected.trim().to_owned();
        let original = &parsed_lines[item.line - 1].2;
        if corrected.is_empty() || corrected == original.trim() {
            continue;
        }
        by_line.insert(item.line, (corrected, item.reason.trim().to_owned()));
    }

    let mut edits = Vec::with_capacity(by_line.len());
    let mut cleaned_text = String::with_capacity(transcript_txt.len());
    for (idx, (prefix, start, orig)) in parsed_lines.iter().enumerate() {
        let line_no = idx + 1;
        if let Some((suggested, reason)) = by_line.get(&line_no) {
            edits.push(AiCleanEdit {
                line: line_no,
                time: start.clone(),
                original: orig.clone(),
                suggested: suggested.clone(),
                reason: reason.clone(),
            });
            cleaned_text.push_str(prefix);
            cleaned_text.push_str(suggested);
        } else {
            cleaned_text.push_str(prefix);
            cleaned_text.push_str(orig);
        }
        cleaned_text.push('\n');
    }

    Ok(AiCleanProposal {
        edits,
        cleaned_text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phantom_lines_disappear_and_the_rest_keep_timestamps() {
        let input = "[00:00:00 --> 00:00:05] Hãy subscribe cho kênh Ghiền Mì Gõ\n\
                     [00:00:05 --> 00:00:09] xem tài hiệu này\n";
        assert_eq!(
            clean_transcript(input),
            "[00:00:05 --> 00:00:09] xem tài liệu này\n"
        );
    }

    #[test]
    fn five_repeats_collapse_and_four_do_not() {
        assert_eq!(collapse_repeats("dạ à à à à à rồi"), "dạ à [à ×5] rồi");
        assert_eq!(collapse_repeats("à à à à rồi"), "à à à à rồi");
    }

    #[test]
    fn repeats_match_across_case_and_punctuation() {
        assert_eq!(collapse_repeats("Dạ, dạ dạ. dạ dạ"), "Dạ, [dạ ×5]");
    }

    #[test]
    fn a_line_without_timestamp_is_still_cleaned() {
        assert_eq!(clean_transcript("tài hiệu\n"), "tài liệu\n");
    }

    #[test]
    fn glossary_replaces_heard_as_phrases_on_word_boundaries() {
        let terms = vec![
            Term {
                correct: "Bravo".into(),
                heard_as: "ba vô, bra vô".into(),
                note: String::new(),
            },
            Term {
                correct: "LÔ-01".into(),
                heard_as: "lô".into(),
                note: String::new(),
            },
        ];
        let input = "[00:00:05 --> 00:00:09] đồng bộ Ba Vô không bị lỗi ở lô này\n";
        assert_eq!(
            clean_transcript_with_glossary(input, &terms),
            "[00:00:05 --> 00:00:09] đồng bộ Bravo không bị lỗi ở LÔ-01 này\n"
        );
    }
}
