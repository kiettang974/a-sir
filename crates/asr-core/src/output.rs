//! Transcript file formats. TXT keeps the old worker's line format so existing habits still work.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::transcriber::Segment;

/// Writes beside and then renames, so a crash never leaves half a file under the real name.
pub(crate) fn save_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(tmp, path)
}

/// "name.ext", then "name (2).ext", "name (3).ext"… One path per extension in each candidate,
/// in order. The caller decides which candidate is free; this only spells the never-overwrite
/// naming in one place.
pub(crate) fn candidates<'a>(
    dir: &'a Path,
    stem: &'a str,
    extensions: &'a [&'a str],
) -> impl Iterator<Item = Vec<PathBuf>> + 'a {
    (1u32..).map(move |n| {
        let base = if n == 1 {
            stem.to_owned()
        } else {
            format!("{stem} ({n})")
        };
        extensions
            .iter()
            .map(|ext| dir.join(format!("{base}.{ext}")))
            .collect()
    })
}

fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

pub fn to_txt(segments: &[Segment]) -> String {
    segments
        .iter()
        .map(|s| {
            format!(
                "[{} --> {}] {}\n",
                clock(s.start_ms),
                clock(s.end_ms),
                s.text
            )
        })
        .collect()
}

pub fn to_srt(segments: &[Segment]) -> String {
    segments
        .iter()
        .enumerate()
        .map(|(i, s)| {
            format!(
                "{}\n{},{:03} --> {},{:03}\n{}\n\n",
                i + 1,
                clock(s.start_ms),
                s.start_ms % 1000,
                clock(s.end_ms),
                s.end_ms % 1000,
                s.text
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg() -> Vec<Segment> {
        vec![Segment {
            start_ms: 3_661_250,
            end_ms: 3_670_000,
            text: "xin chào".into(),
        }]
    }

    #[test]
    fn txt_matches_the_old_worker_line_format() {
        assert_eq!(to_txt(&seg()), "[01:01:01 --> 01:01:10] xin chào\n");
    }

    #[test]
    fn srt_is_numbered_and_keeps_milliseconds() {
        assert_eq!(
            to_srt(&seg()),
            "1\n01:01:01,250 --> 01:01:10,000\nxin chào\n\n"
        );
    }
}
