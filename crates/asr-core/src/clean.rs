//! Rule-based transcript cleanup: known Whisper hallucinations, stutter loops,
//! and optional user glossary replacements (`heard_as` → `correct`).

use unicode_normalization::UnicodeNormalization;

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
