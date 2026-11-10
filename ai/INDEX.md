# INDEX: mục lục source cho agent

> Thêm, đổi tên hoặc xoá file/module thì phải sửa file này trong cùng thay đổi.

## 1. Tổng quan

ASR Studio là app macOS chép lời file ghi âm tiếng Việt ngay trên máy (Rust + Tauri 2 + whisper.cpp qua `whisper-rs`, Metal), rồi dùng nhà cung cấp AI do người dùng chọn (Claude, ChatGPT, Gemini) để biến bản chép lời thành bảng meeting note. Âm thanh không rời máy; chỉ text bản chép lời được gửi, và chỉ đến đúng nhà cung cấp người dùng chọn.

- `crates/asr-core`: toàn bộ logic, không phụ thuộc Tauri.
- `app`: lớp Tauri mỏng, mỗi `#[tauri::command]` chuyển tiếp sang `asr-core`.
- `ui`: TypeScript thuần (không framework), build bằng Vite.

Luồng dữ liệu:

```
thả file / chọn file / chọn từ Ghi âm (voice_memos::list)
        │  invoke("enqueue" | "enqueue_voice_memos")
        ▼
JobRunner::enqueue_titled ── jobs.json (Paths.data)
        │  worker thread, một việc một lúc
        ▼
audio::decode ──► Vec<f32> 16 kHz mono (Symphonia, fallback afconvert)
        ▼
silence_ends + plan_chunks ──► khúc ~15 phút, cắt ở chỗ im lặng
        ▼  mỗi khúc: Transcriber::transcribe (WhisperCpp, VAD Silero)
checkpoint.json (work/<id>/) ── resume sau crash/pause
        ▼
output::to_txt / to_srt ──► <tên>.txt + <tên>.srt (Paths.output, không ghi đè)
        ▼  JobState::Done { txt, srt }; event "queue" lên UI
JobRunner::transcript(id) ──► Transcript (text, lines, write_cleaned, note, save_note)
        ▼  invoke("summarize_job")
Workspace::summarize ──► summary::summarize(&dyn LlmClient = HttpClient, transcript, glossary)
        ▼
Draft { rows, pending } ──► ui/src/note.ts (sửa tại chỗ, nghe lại evidence)
        ▼  invoke("save_note")
<tên>_note.json + <tên>_note.html; "Chép để dán vào email" → clipboard HTML
```

Vị trí file trên máy (đặt trong `app/src/main.rs` `main`):
- `Paths.data` = app data dir: `jobs.json`, `work/<id>/checkpoint.json`, `ai.json`, `api-keys.json` (mode 0600), `glossary.json`.
- `Paths.output` = `~/Documents/ASR Studio`: `.txt`, `.srt`, `_fixed.txt`, `_note.json`, `_note.html`.
- Model: `~/Library/Application Support/asr-studio/models` (`ggml-large-v3-turbo.bin`, `ggml-silero-v5.1.2.bin`).

## 2. Từng file

### crates/asr-core/src

| File | Sở hữu | `pub` chính |
|---|---|---|
| `lib.rs` | Khai báo module, re-export từ `jobs` | `pub use jobs::{Job, JobRunner, JobState, Paths, Queue, Rejected}`; `jobs` là module private |
| `jobs.rs` | Hàng đợi: một worker thread, chia khúc theo im lặng, checkpoint/resume, pause, cancel, giữ/thả engine, đặt tên và ghi file kết quả | `JobState { Queued, Running{percent,eta_s}, Done{txt,srt}, Failed{message}, Cancelled }` + `is_active`; `Job { id, source, title, state }`; `Queue<'a> { jobs, paused, busy }`; `Rejected { AlreadyQueued(id), Unsupported }`; `Paths { data, output }`; `JobRunner::{start(paths, load, on_change), jobs, is_paused, is_busy, enqueue, enqueue_titled, transcript, set_paused, cancel, retry, remove}`. Private nhưng quan trọng: `process`, `run_worker`, `plan_chunks`, `silence_ends`, `output_name`, `write_outputs`, `fingerprint`, hằng `CHUNK_S`/`SEARCH_S`/`MAX_LAST_CHUNK_S`/`MIN_CHUNK_S` |
| `jobs/tests.rs` | Test của `jobs` (fake `Transcriber`, `Rig`) | (test) |
| `transcript.rs` | Bản chép lời đã xong và file đi kèm (`_fixed.txt`, `_note.json`, `_note.html`); parser duy nhất của dòng TXT | `Line { start, text }`; `split_line`; `lines`; `Transcript::{source, text, lines, write_cleaned, note, save_note}` (`new` là `pub(crate)`, chỉ `JobRunner::transcript` tạo) |
| `transcriber.rs` | Seam giữa jobs và engine; adapter whisper.cpp, VAD tự chạy | `SAMPLE_RATE = 16_000`; `Segment { start_ms, end_ms, text }`; `TranscribeError { Cancelled, Engine }`; `Control { cancel, on_progress }`; trait `Transcriber::transcribe`; `WhisperOptions` (mặc định `vi`, beam 1, VAD, pad 400 ms, không timestamp); `WhisperCpp::load` |
| `audio.rs` | Mọi file âm thanh/video → 16 kHz mono f32 (Symphonia + rubato; fallback `afconvert` trên macOS) | `EXTENSIONS`; `is_supported`; `decode` |
| `output.rs` | Định dạng TXT/SRT, ghi atomic, quy tắc tên không ghi đè | `to_txt`, `to_srt`; `pub(crate) save_atomic`, `pub(crate) candidates` (`name`, `name (2)`, ...) |
| `clean.rs` | Làm sạch theo luật: bỏ dòng "ảo giác" kiểu kênh YouTube, sửa chính tả cố định, gộp từ lặp ≥5 | `clean_transcript`; hằng `SPELLING`, `PHANTOM_STARTS`, `MIN_REPEATS` |
| `models.rs` | File model: kiểm tra, tải (resume bằng Range, kiểm SHA-256), nạp engine | `load_engine(dir)` (chính là `load` truyền vào `JobRunner::start`); `are_ready`; `download(dir, on_percent)`; hằng `WHISPER`, `VAD` (url, sha256, bytes) |
| `ai.rs` | Giao tiếp nhà cung cấp AI: wire format từng provider, lỗi, cấu hình, file API key | `Provider { Claude, ChatGpt, Gemini }` + `ALL`, `default_model`; `AiError`; trait `LlmClient::complete(system, user, schema)`; `HttpClient::{new, ping, list_models}`; `AiSettings { default_provider, models }` + `load`, `save`, `model`; `ApiKeys::{at, get, set, remove, hints}`; `key_hint`. Private: `request` (body/header từng provider), `read_text` (đọc reply, truncated/refused), `check_status`, `is_chat_model`, `CLAUDE_WITH_FALLBACKS` |
| `summary.rs` | Prompt, JSON schema, và chuyển câu trả lời thành `Draft` | `Evidence { time, quote, seconds }`; `Row { title, agreed, note, evidence, uncertain }`; `Pending { row, reason }`; `Draft { rows, pending }`; `summarize(&dyn LlmClient, transcript, terms)`. Private: `RULES` (system prompt), `schema()`, `prompt`, `glossary_lines`, `parse`, `seconds` |
| `workspace.rs` | Cấu hình của người dùng trong data dir; cửa vào duy nhất cho AI settings, key, glossary, tóm tắt | `Workspace::{at, ai_settings, save_ai_settings, key_hints, set_key, delete_key, client, summarize, glossary, save_glossary, import_glossary, export_glossary}` |
| `glossary.rs` | Danh sách thuật ngữ đúng (chỉ dùng khi tóm tắt), JSON + CSV có BOM, nhận cả `;` của Excel | `Term { correct, heard_as, note }`; `load`, `save`, `to_csv`, `from_csv`, `merge` |
| `voice_memos.rs` | Đọc app Ghi âm của macOS: SQLite `CloudRecordings.db` (cột dò lúc chạy), fallback quét thư mục | `Recording { path, title, recorded_at, duration_s, on_disk }`; `ListError { NoPermission, Other }`; `folder(home)`; `list(dir)` |

### crates/asr-core/examples

| File | Dùng để |
|---|---|
| `bench.rs` | Đo engine: `bench <model> <vad> <wav> <reference.txt> <out.txt> [--beam N] [--no-vad] [--pad MS] [--ts]`, in tốc độ và độ lệch từ so với bản tham chiếu |
| `run_job.rs` | Chạy engine thật qua `JobRunner` thật: `run_job <models dir> <work dir> <audio file>` (env `HOLD_S` để giữ process đo RAM) |
| `list_memos.rs` | In số bản ghi `voice_memos::list` thấy được |

### app

`app/src/main.rs`: struct `App { runner, workspace, models_dir, output_dir }` được `manage`; `main` dựng đường dẫn, `JobRunner::start` với `models::load_engine` và callback phát event `"queue"` + `KeepAwake` (chạy `caffeinate` khi `busy`). `off_thread` chạy việc mạng/blocking trên `spawn_blocking`.

| Command | Chuyển tiếp sang |
|---|---|
| `snapshot` | `runner.jobs/is_paused/is_busy`, `models::are_ready`, `audio::EXTENSIONS` |
| `enqueue(paths)` | `runner.enqueue` (trả danh sách lý do bị từ chối) |
| `list_voice_memos` | `voice_memos::folder` + `voice_memos::list` → `MemoList { ok, noPermission, failed }` |
| `enqueue_voice_memos(choices)` | `runner.enqueue_titled(path, Some(title))` |
| `open_full_disk_settings` | `open x-apple.systempreferences:...Privacy_AllFiles` (không qua core) |
| `relaunch` | `AppHandle::restart` |
| `cancel(id)` / `retry(id)` / `remove(id)` / `set_paused(paused)` | `runner.cancel/retry/remove/set_paused` |
| `read_transcript(id)` | `runner.transcript(id)?.text()` + `transcript::lines` |
| `clean_transcript(id)` | `Transcript::write_cleaned` |
| `ai_settings` | `workspace.ai_settings` + `workspace.key_hints` → `AiView` (key thật không lên UI) |
| `save_ai_settings(settings)` | `workspace.save_ai_settings` |
| `set_api_key(provider,key)` / `delete_api_key(provider)` | `workspace.set_key/delete_key` |
| `test_provider(provider, model)` | `workspace.client(...).ping()` (off_thread) |
| `list_models(provider)` | `workspace.client(...).list_models()` (off_thread) |
| `get_glossary` / `save_glossary(terms)` | `workspace.glossary/save_glossary` |
| `import_glossary(terms, path)` / `export_glossary(terms, path)` | `workspace.import_glossary/export_glossary` |
| `summarize_job(id, provider)` | `runner.transcript(id)?.text()` → `workspace.summarize` (off_thread) |
| `save_note(id, draft, html)` / `load_note(id)` | `Transcript::save_note` / `Transcript::note` |
| `allow_job_audio(id)` | `Transcript::source` + `asset_protocol_scope().allow_file` |
| `download_models` | `models::download`, phát event `"model-progress"` (off_thread) |

Event phát ra: `"queue"` (payload `Queue`), `"model-progress"` (u8 percent).

- `app/capabilities/default.json`: quyền plugin cho cửa sổ `main` (`core:default`, `dialog:allow-open/save/ask/message`, `clipboard-manager:allow-write-text/html`, `opener:allow-reveal-item-in-dir`). Command riêng của app không cần khai báo ở đây; chỉ quyền plugin.
- `app/tauri.conf.json`: identifier, cửa sổ, CSP (`media-src asset:` cho trình phát), `assetProtocol.scope: []` (mở từng file lúc chạy qua `allow_job_audio`), lệnh build UI (`pnpm dev`/`pnpm build` ở `../ui`).

### Gốc repo

- `.cargo/config.toml`: đặt sẵn `GGML_MACHINE_SUPPORTS_{i8mm,sve,sme}=0`. Bước dò CPU của CMake whisper.cpp chạy thử chương trình cho từng tính năng; trên macOS 27.2 chương trình dò i8mm/SVE/SME quay mãi thay vì báo lỗi lệnh, làm build treo ở `Compiling whisper-rs-sys`. Đổi máy (chip có i8mm thật) thì xem lại file này.

### ui/src

| File | Sở hữu | Hàm chính | `invoke` |
|---|---|---|---|
| `main.ts` | Entry: state toàn cục, layout hai cột (hàng đợi + pane phải), màn tải model, chọn từ Ghi âm, đọc bản chép lời, kéo thả | `main` (snapshot + `listen("queue")`, `listen("model-progress")`, `onDragDropEvent`), `render`, `queuePane`, `jobRow`, `select`, `addFiles`, `enqueue`, `reportSkipped`, `openMemos`, `memosPane`, `memoRow`, `refreshMemoControls`, `readerPane`, `firstRun`, `startDownload`, `rightPane` (chọn `reader`/`memos`/`settings`/`note`) | `snapshot`, `enqueue`, `cancel`, `retry`, `remove`, `set_paused`, `list_voice_memos`, `enqueue_voice_memos`, `open_full_disk_settings`, `relaunch`, `read_transcript`, `clean_transcript`, `download_models` |
| `note.ts` | Pane bảng tóm tắt: form tạo, bảng sửa tại chỗ (contentEditable), mục "Cần xác nhận", nghe lại evidence, tự lưu, HTML email | `notePane(id, name, links)` (bên trong: `showForm`, `drawForm`, `showNote`, `draw`, `save`, `play`), `emailHtml` (bảng gửi đi, style inline), `cleanHtml` (whitelist b/i/br/a http), `plainText`, `fromModelText` (`**bold**`, xuống dòng), `handlePaste`, `editable`; hằng `DEFAULT_INTRO`, `SAVE_DELAY_MS` | `ai_settings`, `summarize_job`, `save_note`, `load_note`, `allow_job_audio` |
| `settings.ts` | Pane Cấu hình: nhà cung cấp AI, key, model, glossary | `settingsPane`, `aiSection`, `providerBlock`, `glossarySection`; export `Provider`, `PROVIDERS`, `PROVIDER_NAME`, `AiView` | `ai_settings`, `get_glossary`, `save_ai_settings`, `set_api_key`, `delete_api_key`, `list_models`, `test_provider`, `save_glossary`, `import_glossary`, `export_glossary` |
| `dom.ts` | Helper tạo element, mọi text qua `textContent` | `el(tag, props, ...children)` | - |
| `style.css` | Toàn bộ CSS (biến màu/font ở `:root`; khối Queue, Reader, Invite, Memos, Settings, Note) | - | - |

Type phía UI viết tay, phải khớp serde của Rust: `JobState`/`Job`/`Recording`/`MemoList`/`Snapshot` trong `main.ts`, `Draft`/`Row`/`Evidence` trong `note.ts`, `AiView`/`Term`/`Provider` trong `settings.ts`.

## 3. Muốn làm X thì xem đâu

| Việc | File |
|---|---|
| Đổi prompt hoặc schema bảng tóm tắt | `crates/asr-core/src/summary.rs` (`RULES`, `schema`, `prompt`, `parse`); thêm field thì sửa `Row`/`Draft` và type trong `ui/src/note.ts` |
| Thêm nhà cung cấp AI | `ai.rs` (`Provider`, `ALL`, `default_model`, `base_url`, nhánh trong `request`, `read_text`, `list_models`); `ui/src/settings.ts` (`Provider`, `PROVIDERS`, `PROVIDER_NAME`) |
| Đổi model mặc định / fallback của Claude | `ai.rs` (`Provider::default_model`, `CLAUDE_WITH_FALLBACKS`) |
| Đổi cách lưu API key / cấu hình AI | `ai.rs` (`ApiKeys`, `AiSettings`), tên file ở `workspace.rs` |
| Đổi tên file kết quả / luật không ghi đè | `jobs.rs` (`output_name`, `write_outputs`), `output.rs` (`candidates`), `transcript.rs` (`write_cleaned`, `sidecar`) |
| Đổi định dạng TXT/SRT | `output.rs` (`to_txt`, `to_srt`) và parser `transcript.rs` (`split_line`) phải đổi cùng; `clean.rs` dùng `split_line` |
| Đổi cách chia khúc | `jobs.rs` (`plan_chunks`, `silence_ends`, hằng `CHUNK_S`...) |
| Đổi tham số whisper / VAD | `transcriber.rs` (`WhisperOptions::default`, `speech_spans`, `merge_spans`, `transcribe`) |
| Thêm định dạng âm thanh | `audio.rs` (`EXTENSIONS`, feature của `symphonia` trong `crates/asr-core/Cargo.toml`); UI lấy danh sách qua `snapshot` |
| Đổi tải model / đổi model | `models.rs` (`WHISPER`, `VAD`, `download`, `fetch`); thư mục model đặt ở `app/src/main.rs` `main`; màn tải ở `main.ts` `firstRun` |
| Đổi cách đọc app Ghi âm | `voice_memos.rs` (`read_database`, `scan_folder`, `folder`); UI ở `main.ts` `memosPane` |
| Đổi luật làm sạch | `clean.rs` |
| Đổi glossary / CSV | `glossary.rs`; UI ở `settings.ts` `glossarySection`; cách đưa vào prompt ở `summary.rs` `glossary_lines` |
| Đổi HTML email / bảng gửi đi | `ui/src/note.ts` (`emailHtml`, `cleanHtml`, `plainText`); HTML lưu file cũng dựng từ `emailHtml` trong `save` |
| Đổi trạng thái việc / pause / cancel / retry | `jobs.rs` (`JobRunner` methods, `run_worker`, `process`); hiển thị ở `main.ts` `statusText`, `jobRow` |
| Thêm Tauri command | 1) logic ở `asr-core` (thường qua `JobRunner`, `Transcript` hoặc `Workspace`); 2) `#[tauri::command]` trong `app/src/main.rs` và thêm vào `generate_handler!`; 3) `invoke("tên")` trong `ui/src/*.ts` với type khớp serde (field camelCase khi struct có `rename_all`); 4) chỉ khi dùng plugin mới (dialog, clipboard, opener...) mới thêm quyền vào `app/capabilities/default.json`; việc nặng/mạng bọc `off_thread` |
| Cho UI đọc thêm file trên đĩa | `allow_job_audio` trong `main.rs` (asset scope), CSP ở `app/tauri.conf.json` |
| Đổi giao diện | `ui/src/style.css`; bố cục ở `main.ts` `render` |

## 4. Seam và test

- `Transcriber` (`transcriber.rs`): adapter thật `WhisperCpp`; fake trong test là `Fake` và `Counted` ở `jobs/tests.rs`. `JobRunner::start` nhận closure `load` trả `Box<dyn Transcriber>`, nên test không cần model.
- `LlmClient` (`ai.rs`): adapter thật `HttpClient`; fake là `Fake` trong `summary.rs` tests. Test của `HttpClient` dùng server TCP một lần (`serve`, `serve_many`) qua `with_base_url`.
- Test (đều là `#[cfg(test)] mod tests` trong file, trừ jobs): `jobs/tests.rs`, `transcript.rs`, `transcriber.rs` (`merge_spans`), `audio.rs`, `output.rs`, `clean.rs`, `models.rs` (server HTTP giả cho resume/hash), `ai.rs`, `summary.rs`, `workspace.rs`, `glossary.rs`, `voice_memos.rs` (SQLite giả).
- Chạy: `cargo test -p asr-core`; UI: `pnpm --dir ui build` (chạy `tsc --noEmit` rồi `vite build`).
- Không có test: `ui/src/*` và `app/src/main.rs`. Engine thật chỉ được thử qua `examples/run_job.rs` và `examples/bench.rs`.

## 5. Tài liệu khác

- `ai/GLOSSARY.md`: thuật ngữ domain.
- `ai/domain.md`: cách dùng tài liệu domain.
- `ai/issue-tracker.md`: issue nằm ở `.scratch/<feature>/`.
- `ai/triage-labels.md`: năm nhãn triage.
- `ai/adr/`: ADR (có thể chưa tồn tại).
- `README.md` (gốc repo): cách build, nơi lưu model và kết quả.
- `AGENTS.md` (gốc repo): điểm vào cho agent.
