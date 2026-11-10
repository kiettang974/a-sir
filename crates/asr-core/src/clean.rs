//! Rule-based transcript cleanup: known Whisper hallucinations and stutter loops.

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

/// Phantom lines are dropped outright; the old script left blank lines behind.
pub fn clean_transcript(txt: &str) -> String {
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
            Some(format!("{prefix}{}\n", collapse_repeats(&fixed)))
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
}
