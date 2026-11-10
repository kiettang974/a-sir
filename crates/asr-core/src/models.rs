//! The speech engine's model files: whether they're here, fetching them on first run, and
//! loading the engine from them. Files are checked by hash, so a broken download can't load.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::transcriber::{TranscribeError, Transcriber, WhisperCpp, WhisperOptions};

struct Model {
    file: &'static str,
    url: &'static str,
    sha256: &'static str,
    bytes: u64,
}

const WHISPER: Model = Model {
    file: "ggml-large-v3-turbo.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin",
    sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
    bytes: 1_624_555_275,
};
const VAD: Model = Model {
    file: "ggml-silero-v5.1.2.bin",
    url: "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin",
    sha256: "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf",
    bytes: 885_098,
};

fn whisper_path(dir: &Path) -> PathBuf {
    dir.join(WHISPER.file)
}

fn vad_path(dir: &Path) -> PathBuf {
    dir.join(VAD.file)
}

/// The engine, from the models in `dir`. This is the `load` that `JobRunner::start` takes.
pub fn load_engine(dir: &Path) -> Result<Box<dyn Transcriber>, TranscribeError> {
    if !are_ready(dir) {
        return Err(TranscribeError::Engine(
            "chưa tải đủ model nhận dạng giọng nói, hãy mở lại app để tải".into(),
        ));
    }
    let engine = WhisperCpp::load(&whisper_path(dir), &vad_path(dir), WhisperOptions::default())?;
    Ok(Box::new(engine))
}

/// Checked by size only: hashing 1.6 GB on every launch would be slow, and
/// a file only ever gets this name after `download` verified its hash.
pub fn are_ready(dir: &Path) -> bool {
    [&WHISPER, &VAD]
        .iter()
        .all(|m| fs::metadata(dir.join(m.file)).is_ok_and(|meta| meta.len() == m.bytes))
}

/// Downloads whatever is missing and resumes an interrupted file.
pub fn download(dir: &Path, on_percent: &dyn Fn(u8)) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let total = WHISPER.bytes + VAD.bytes;
    let mut finished = 0;
    for model in [&WHISPER, &VAD] {
        fetch(model, model.url, dir, &|got| {
            on_percent(((finished + got) * 100 / total) as u8)
        })?;
        finished += model.bytes;
    }
    Ok(())
}

fn fetch(model: &Model, url: &str, dir: &Path, on_bytes: &dyn Fn(u64)) -> Result<(), String> {
    let target = dir.join(model.file);
    if fs::metadata(&target).is_ok_and(|m| m.len() == model.bytes) {
        return Ok(());
    }
    let part = dir.join(format!("{}.part", model.file));
    let mut have = fs::metadata(&part).map_or(0, |m| m.len());
    if have > model.bytes {
        fs::remove_file(&part).map_err(|e| e.to_string())?;
        have = 0;
    }
    if have < model.bytes {
        let client = reqwest::blocking::Client::builder()
            .timeout(None)
            .build()
            .map_err(|e| e.to_string())?;
        let mut response = client
            .get(url)
            .header("Range", format!("bytes={have}-"))
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("không tải được model: {e}"))?;
        // A server that ignores Range sends the whole file again.
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            have = 0;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(have > 0)
            .truncate(have == 0)
            .open(&part)
            .map_err(|e| e.to_string())?;
        let mut buf = vec![0; 1 << 20];
        loop {
            let n = response
                .read(&mut buf)
                .map_err(|e| format!("mất kết nối khi tải model: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            have += n as u64;
            on_bytes(have);
        }
    }
    let actual = sha256(&part).map_err(|e| e.to_string())?;
    if actual != model.sha256 {
        fs::remove_file(&part).ok();
        return Err(format!("model tải về bị lỗi ({}), hãy thử lại", model.file));
    }
    fs::rename(&part, &target).map_err(|e| e.to_string())
}

fn sha256(path: &Path) -> io::Result<String> {
    let mut hasher = Sha256::new();
    io::copy(&mut File::open(path)?, &mut hasher)?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_a_known_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc");
        fs::write(&path, "abc").unwrap();
        assert_eq!(
            sha256(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::thread;

    const CONTENT: &[u8] = b"model bytes, small for the test";

    fn model() -> Model {
        Model {
            file: "m.bin",
            url: "unused",
            // sha256 of CONTENT
            sha256: Box::leak(sha256_of(CONTENT).into_boxed_str()),
            bytes: CONTENT.len() as u64,
        }
    }

    fn sha256_of(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Serves `body` once. Honours "Range: bytes=N-" with a 206 unless `ignore_range`.
    /// Returns the URL and what Range header, if any, was asked for.
    fn serve(body: &'static [u8], ignore_range: bool) -> (String, thread::JoinHandle<Option<u64>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/m.bin", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut range = None;
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
                if let Some(v) = line.to_lowercase().strip_prefix("range: bytes=") {
                    range = v.trim().trim_end_matches('-').parse::<u64>().ok();
                }
            }
            let (status, part) = match range {
                Some(from) if !ignore_range => ("206 Partial Content", &body[from as usize..]),
                _ => ("200 OK", body),
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                part.len()
            )
            .unwrap();
            stream.write_all(part).unwrap();
            range
        });
        (url, handle)
    }

    #[test]
    fn an_interrupted_download_resumes_where_it_stopped() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("m.bin.part"), &CONTENT[..10]).unwrap();
        let (url, server) = serve(CONTENT, false);
        fetch(&model(), &url, dir.path(), &|_| {}).unwrap();
        assert_eq!(server.join().unwrap(), Some(10));
        assert_eq!(fs::read(dir.path().join("m.bin")).unwrap(), CONTENT);
        assert!(!dir.path().join("m.bin.part").exists());
    }

    #[test]
    fn a_server_that_ignores_range_restarts_the_file_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("m.bin.part"), &CONTENT[..10]).unwrap();
        let (url, server) = serve(CONTENT, true);
        fetch(&model(), &url, dir.path(), &|_| {}).unwrap();
        server.join().unwrap();
        assert_eq!(fs::read(dir.path().join("m.bin")).unwrap(), CONTENT);
    }

    #[test]
    fn a_download_with_the_wrong_hash_is_thrown_away() {
        let dir = tempfile::tempdir().unwrap();
        let (url, server) = serve(b"other bytes of the same length!", false);
        assert_eq!(b"other bytes of the same length!".len(), CONTENT.len());
        let error = fetch(&model(), &url, dir.path(), &|_| {}).unwrap_err();
        server.join().unwrap();
        assert!(error.contains("bị lỗi"), "{error}");
        assert!(!dir.path().join("m.bin").exists());
        assert!(!dir.path().join("m.bin.part").exists(), "next try starts fresh");
    }

    #[test]
    fn the_engine_will_not_load_without_the_models() {
        let dir = tempfile::tempdir().unwrap();
        let Err(error) = load_engine(dir.path()) else {
            panic!("loaded from an empty folder")
        };
        assert!(error.to_string().contains("chưa tải đủ model"));
    }

    #[test]
    fn a_wrong_size_model_is_not_ready() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!are_ready(dir.path()));
        fs::write(whisper_path(dir.path()), "short").unwrap();
        assert!(!are_ready(dir.path()));
    }
}
