//! Loopback-only HTTP/1.1 Range server for playing job recordings in macOS WKWebView.
//!
//! WKWebView's AVFoundation media pipeline has well-known issues streaming/seeking audio over
//! custom `WKURLSchemeHandler` schemes (`asset://`). Serving only explicitly registered job
//! files over `127.0.0.1` with a per-session secret token and proper `206 Partial Content`
//! responses keeps the same security boundary while making `<audio>` seeking reliable.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub struct AudioServer {
    port: u16,
    token: String,
    allowed: Arc<Mutex<HashMap<u64, PathBuf>>>,
}

impl AudioServer {
    pub fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let token = generate_token();
        let allowed: Arc<Mutex<HashMap<u64, PathBuf>>> = Arc::default();

        let thread_allowed = Arc::clone(&allowed);
        let thread_token = token.clone();
        std::thread::Builder::new()
            .name("audio-server".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let allowed = Arc::clone(&thread_allowed);
                    let token = thread_token.clone();
                    std::thread::spawn(move || {
                        let _ = handle_connection(stream, &allowed, &token);
                    });
                }
            })?;

        Ok(Self {
            port,
            token,
            allowed,
        })
    }

    /// Registers `path` for `job_id` and returns its loopback playback URL.
    pub fn register(&self, job_id: u64, path: PathBuf) -> String {
        self.allowed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(job_id, path);
        format!(
            "http://127.0.0.1:{}/audio/{}?token={}",
            self.port, job_id, self.token
        )
    }
}

fn generate_token() -> String {
    let mut bytes = [0u8; 16];
    if let Ok(mut f) = File::open("/dev/urandom")
        && f.read_exact(&mut bytes).is_ok()
    {
        return bytes.iter().map(|b| format!("{b:02x}")).collect();
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}{:x}", std::process::id(), nanos)
}

fn handle_connection(
    mut stream: TcpStream,
    allowed: &Arc<Mutex<HashMap<u64, PathBuf>>>,
    expected_token: &str,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    if method != "GET" && method != "HEAD" {
        return write_status(&mut stream, "405 Method Not Allowed");
    }

    let mut range_header: Option<String> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("range")
        {
            range_header = Some(value.trim().to_owned());
        }
    }

    let Some((path_part, query)) = target.split_once('?') else {
        return write_status(&mut stream, "403 Forbidden");
    };
    let token_ok = query
        .split('&')
        .any(|kv| kv.strip_prefix("token=") == Some(expected_token));
    if !token_ok {
        return write_status(&mut stream, "403 Forbidden");
    }
    let Some(id_str) = path_part.strip_prefix("/audio/") else {
        return write_status(&mut stream, "404 Not Found");
    };
    let Ok(job_id) = id_str.parse::<u64>() else {
        return write_status(&mut stream, "404 Not Found");
    };

    let file_path = allowed
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&job_id)
        .cloned();
    let Some(file_path) = file_path else {
        return write_status(&mut stream, "404 Not Found");
    };

    let Ok(mut file) = File::open(&file_path) else {
        return write_status(&mut stream, "404 Not Found");
    };
    let total_len = file.metadata()?.len();
    let mime = mime_for(&file_path);

    if let Some(range) = range_header {
        let Some((start, end)) = parse_range(&range, total_len) else {
            let header = format!(
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{total_len}\r\nConnection: close\r\n\r\n"
            );
            return stream.write_all(header.as_bytes());
        };
        let chunk_len = end - start + 1;
        let header = format!(
            "HTTP/1.1 206 Partial Content\r\n\
             Content-Type: {mime}\r\n\
             Accept-Ranges: bytes\r\n\
             Content-Range: bytes {start}-{end}/{total_len}\r\n\
             Content-Length: {chunk_len}\r\n\
             Cache-Control: no-store\r\n\
             Connection: close\r\n\r\n"
        );
        stream.write_all(header.as_bytes())?;
        if method == "GET" {
            file.seek(SeekFrom::Start(start))?;
            stream_bytes(&mut file, &mut stream, chunk_len)?;
        }
    } else {
        let header = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: {mime}\r\n\
             Accept-Ranges: bytes\r\n\
             Content-Length: {total_len}\r\n\
             Cache-Control: no-store\r\n\
             Connection: close\r\n\r\n"
        );
        stream.write_all(header.as_bytes())?;
        if method == "GET" {
            stream_bytes(&mut file, &mut stream, total_len)?;
        }
    }
    Ok(())
}

fn stream_bytes(file: &mut File, stream: &mut TcpStream, mut remaining: u64) -> std::io::Result<()> {
    let mut buf = [0u8; 64 * 1024];
    while remaining > 0 {
        let to_read = (remaining as usize).min(buf.len());
        let n = file.read(&mut buf[..to_read])?;
        if n == 0 {
            break;
        }
        stream.write_all(&buf[..n])?;
        remaining -= n as u64;
    }
    Ok(())
}

fn write_status(stream: &mut TcpStream, status: &str) -> std::io::Result<()> {
    let resp = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    stream.write_all(resp.as_bytes())
}

fn parse_range(header: &str, total_len: u64) -> Option<(u64, u64)> {
    if total_len == 0 {
        return None;
    }
    let spec = header.strip_prefix("bytes=")?.split(',').next()?.trim();
    let (start_s, end_s) = spec.split_once('-')?;
    if start_s.is_empty() {
        let suffix: u64 = end_s.parse().ok()?;
        if suffix == 0 {
            return None;
        }
        let start = total_len.saturating_sub(suffix);
        return Some((start, total_len - 1));
    }
    let start: u64 = start_s.parse().ok()?;
    if start >= total_len {
        return None;
    }
    let end = if end_s.is_empty() {
        total_len - 1
    } else {
        end_s.parse::<u64>().ok()?.min(total_len - 1)
    };
    (start <= end).then_some((start, end))
}

fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("m4a" | "mp4" | "m4b") => "audio/mp4",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("aac") => "audio/aac",
        Some("caf") => "audio/x-caf",
        Some("aif" | "aiff") => "audio/aiff",
        Some("flac") => "audio/flac",
        Some("ogg" | "oga") => "audio/ogg",
        Some("opus") => "audio/opus",
        Some("webm") => "audio/webm",
        Some("mov") => "video/quicktime",
        _ => "application/octet-stream",
    }
}
