# Bản đồ mã nguồn ASR Studio

Cập nhật bản đồ khi thêm, đổi tên hoặc xoá mô-đun hay điểm vào được mô tả ở đây.

## 1. Tổng quan

ASR Studio là app macOS chép lời file ghi âm tiếng Việt ngay trên máy (Rust + Tauri 2 + whisper.cpp qua `whisper-rs`, Metal), rồi dùng nhà cung cấp AI do người dùng chọn (Claude, ChatGPT, Gemini) để biến bản chép lời thành bảng meeting note. Âm thanh không rời máy; khi người dùng yêu cầu tóm tắt, app gửi bản chép lời và danh sách thuật ngữ đến nhà cung cấp đã chọn.

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
- `Paths.data` = `~/Library/Application Support/com.asir.app`: `jobs.json`, `work/<id>/checkpoint.json`, `asr.json`, `ai.json`, `api-keys.json` (mode 0600), `glossary.json`.
- `Paths.output` = `~/Documents/ASR Studio`: `.txt`, `.srt`, `_fixed.txt`, `_note.json`, `_note.html`.
- Model: `~/Library/Application Support/asr-studio/models` (`ggml-large-v3-turbo.bin`, `ggml-large-v3.bin`, `ggml-phowhisper-medium-q5_0.bin`, `qwen3-asr-1.7b/`, `ggml-silero-v5.1.2.bin`).

## 2. Từng file

### crates/asr-core/src

| File | Sở hữu | `pub` chính |
|---|---|---|
| `lib.rs` | Khai báo module, re-export từ `jobs` | `pub use jobs::{Job, JobRunner, JobState, Paths, Queue, Rejected}`; `jobs` là module private |
| `jobs.rs` | Hàng đợi: một worker thread, chia khúc theo im lặng, checkpoint/resume gắn `model` theo job, pause, cancel, giữ/thả/đổi engine theo `model_id`, đặt tên và ghi file kết quả | `JobState { Queued, Running{percent,eta_s}, Done{txt,srt}, Failed{message}, Cancelled }` + `is_active`; `Job { id, source, title, model, state }`; `Queue<'a> { jobs, paused, busy }`; `Rejected { AlreadyQueued(id), Unsupported }`; `Paths { data, output }`; `JobRunner::{start(paths, default_model, load, on_change), default_model, set_default_model, jobs, is_paused, is_busy, enqueue, enqueue_titled, transcript, set_paused, cancel, retry, remove}`. Private nhưng quan trọng: `process`, `run_worker`, `plan_chunks`, `silence_ends`, `output_name`, `write_outputs`, `fingerprint`, hằng `CHUNK_S`/`SEARCH_S`/`MAX_LAST_CHUNK_S`/`MIN_CHUNK_S` |
| `jobs/tests.rs` | Test của `jobs` (fake `Transcriber`, `Rig`) | (test) |
| `transcript.rs` | Bản chép lời đã xong và file đi kèm (`_fixed.txt`, `_note.json`, `_note.html`); parser chuẩn cho dòng TXT | `Line { start, text }`; `split_line`; `lines`; `Transcript::{source, text, lines, write_cleaned, note, save_note}` (`new` là `pub(crate)`, chỉ `JobRunner::transcript` tạo) |
| `transcriber.rs` | Seam giữa jobs và engine; adapter whisper.cpp, VAD tự chạy | `SAMPLE_RATE = 16_000`; `Segment { start_ms, end_ms, text }`; `TranscribeError { Cancelled, Engine }`; `Control { cancel, on_progress }`; trait `Transcriber::transcribe`; `WhisperOptions` (mặc định `vi`, giải mã greedy (beam 1), Silero VAD, pad 400 ms, mốc đoạn lấy từ VAD); `WhisperCpp::load` |
| `audio.rs` | Mọi file âm thanh/video → 16 kHz mono f32 (Symphonia + rubato; fallback `afconvert` trên macOS) | `EXTENSIONS`; `is_supported`; `decode` |
| `output.rs` | Định dạng TXT/SRT, ghi atomic, quy tắc tên không ghi đè | `to_txt`, `to_srt`; `pub(crate) save_atomic`, `pub(crate) candidates` (`name`, `name (2)`, ...) |
| `clean.rs` | Làm sạch theo luật: bỏ dòng "ảo giác" kiểu kênh YouTube, sửa chính tả cố định, gộp từ lặp ≥5 | `clean_transcript`; hằng `SPELLING`, `PHANTOM_STARTS`, `MIN_REPEATS` |
| `models.rs` | Danh mục model chép lời cục bộ (`CATALOG`) và `ModelStore`: kiểm tra, tải (single-flight lock, resume bằng Range, kiểm SHA-256), validate chọn/xoá, nạp engine theo `model_id` | `DEFAULT_MODEL_ID`, `ModelSpec`, `CATALOG`, `ModelStore::{new, is_ready, are_ready, validate_select, load, download, delete}`, `find`, `default_spec` |
| `ai.rs` | Giao tiếp nhà cung cấp AI: wire format từng provider, lỗi, cấu hình (gồm văn phong tóm tắt nhúng offline từ `prompts/vietnamese-humanizer/skills/`), file API key | `Provider { Claude, ChatGpt, Gemini }` + `ALL`, `default_model`; `SummaryStyle` (`label`, `prompt_rules`, `skill_guide`, `preservation_rules`); `AiError`; trait `LlmClient::complete(system, user, schema)`; `HttpClient::{new, ping, list_models}`; `AiSettings { default_provider, models, summary_style }` + `load`, `save`, `model`; `ApiKeys::{at, get, set, remove, hints}`; `key_hint`. Private: `request` (body/header từng provider), `read_text` (đọc reply, truncated/refused), `check_status`, `is_chat_model`, `CLAUDE_WITH_FALLBACKS` |
| `summary.rs` | Prompt, JSON schema theo BA-Kit + văn phong tóm tắt, và chuyển câu trả lời thành `Draft` | `Evidence { time, quote, seconds }`; `Row { title, agreed, note, evidence, uncertain, fields }`; `ActionItem { task, owner, deadline, missing, evidence }`; `Pending { row, reason }`; `Draft { discussion, rows, actions, pending }`; `summarize`, `summarize_with_style`, `summarize_with_options` |
| `deliverable.rs` | Phần chung OOXML cho xuất biên bản Excel/Word: `TemplateFormat`, `MeetingRow`, `escape_xml`, `resolve_column_value`, `patch_zip_entry`, `validate_template`, `inspect_headers` | `TemplateFormat { Excel, Docx }`; `MeetingRow { title, agreed, note, fields }`; `validate_template`; `inspect_headers` |
| `diagram.rs` | Tạo sơ đồ quy trình từ toàn bộ bản chép lời bằng Gemini: prompt, JSON schema, kiểm tra cấu trúc (đủ dữ kiện, nút/đường nối hợp lệ, điểm cần xác nhận) | `NodeKind`, `DiagramNode`, `DiagramEdge`, `DiagramUncertainty`, `ProcessDiagram`, `DiagramError`, `generate`, `parse_and_validate` |
| `excel.rs` | Xuất biên bản `.xlsx` theo mẫu mặc định (`mau-bien-ban-hop.xlsx`) hoặc file `.xlsx` mẫu người dùng import; mọi ô ghi dạng `inlineStr` (text) | `DEFAULT_TEMPLATE`; `inspect_headers`; `validate_template`; `export` |
| `docx.rs` | Xuất biên bản `.docx` theo bố cục mặc định hoặc file `.docx` mẫu người dùng import | `inspect_headers`; `validate_template`; `export` |
| `workspace.rs` | Cấu hình của người dùng trong data dir; điểm vào cho model chép lời cục bộ, cấu hình AI, API key, glossary, mẫu biên bản (`TemplateFormat`) và tóm tắt | `Workspace::{at, asr_model, set_asr_model, resource_profile, set_resource_profile, template_name, template_columns, import_template, reset_template, active_template_columns, export_excel, export_docx, ai_settings, save_ai_settings, key_hints, set_key, delete_key, client, summarize, glossary, save_glossary, import_glossary, export_glossary}` |
| `glossary.rs` | Danh sách thuật ngữ đúng (chỉ dùng khi tóm tắt), JSON + CSV có BOM, nhận cả `;` của Excel | `Term { correct, heard_as, note }`; `load`, `save`, `to_csv`, `from_csv`, `merge` |
| `voice_memos.rs` | Đọc app Ghi âm của macOS: SQLite `CloudRecordings.db` (cột dò lúc chạy), fallback quét thư mục | `Recording { path, title, recorded_at, duration_s, on_disk }`; `ListError { NoPermission, Other }`; `folder(home)`; `list(dir)` |

### crates/asr-core/examples

| File | Dùng để |
|---|---|
| `bench.rs` | Đo engine: `bench <model> <vad> <wav> <reference.txt> <out.txt> [--beam N] [--no-vad] [--pad MS] [--ts]`, in tốc độ và độ lệch từ so với bản tham chiếu do engine cũ tạo |
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
| `probe_accessible_models(provider, candidates, model)` | Thử model đang chọn và tối đa năm model khác. Trả danh sách đã thử và danh sách gọi thành công (off_thread). |
| `get_glossary` / `save_glossary(terms)` | `workspace.glossary/save_glossary` |
| `import_glossary(terms, path)` / `export_glossary(terms, path)` | `workspace.import_glossary/export_glossary` |
| `summarize_job(id, provider)` | `runner.transcript(id)?.text()` → `workspace.summarize` (off_thread) |
| `save_note(id, draft, html)` / `load_note(id)` | `Transcript::save_note` / `Transcript::note` |
| `generate_diagram(id)` / `load_diagram(id)` / `save_diagram(id, diagram)` / `export_diagram_file(path, bytes)` | `Workspace::generate_diagram` (off_thread, chỉ gửi bản chép lời qua Gemini), `Transcript::diagram` / `save_diagram` (`_diagram.json`), ghi tệp PNG/`.excalidraw` |
| `excel_template` / `import_excel_template(path)` / `reset_excel_template` / `export_note_excel(rows, path)` | `workspace.excel_template_name/import_excel_template/reset_excel_template/export_excel` |
| `allow_job_audio(id)` | `Transcript::source` + kiểm tra quyền đọc file + đăng ký vào `audio_server::AudioServer` (`http://127.0.0.1:<port>/audio/<id>?token=...`, hỗ trợ HTTP Range 206 cho WKWebView) |
| `download_models` | `models::download_model` (model đang chọn), phát event `"model-progress"` (off_thread) |
| `local_models` | `models::CATALOG` + `models::is_ready` + `workspace.asr_model` → `LocalModelsView` |
| `download_local_model(model_id)` | `models::download_model`, phát event `"local-model-progress"` (off_thread) |
| `select_local_model(model_id)` | kiểm `models::is_ready` → `workspace.set_asr_model` + `runner.set_default_model` |

Event phát ra: `"queue"` (payload `Queue`), `"model-progress"` (u8 percent), `"local-model-progress"` (`{ modelId, percent }`).

- `app/capabilities/default.json`: quyền plugin cho cửa sổ `main` (`core:default`, `dialog:allow-open/save/ask/message`, `clipboard-manager:allow-write-text/html`, `opener:allow-reveal-item-in-dir`). Command riêng của app không cần khai báo ở đây; chỉ quyền plugin.
- `app/tauri.conf.json`: identifier, cửa sổ, CSP (`media-src asset:` cho trình phát), `assetProtocol.scope: []` (mở từng file lúc chạy qua `allow_job_audio`), lệnh build UI (`pnpm dev`/`pnpm build` ở `../ui`).

### Preview giao diện

Nếu checkout có `ui/mock/`, chạy `pnpm --dir ui preview:mock`. Cấu hình `ui/vite.preview.config.ts` thay các API Tauri bằng mock và phục vụ fixture giả lập trên trình duyệt. Dùng bản app macOS để kiểm tra xử lý âm thanh, quyền hệ điều hành và IPC thật.

### Gốc repo

- `.cargo/config.toml`: đặt sẵn `GGML_MACHINE_SUPPORTS_{i8mm,sve,sme}=0`. Bước dò CPU của CMake whisper.cpp chạy thử chương trình cho từng tính năng; trên macOS 27.2 chương trình dò i8mm/SVE/SME quay mãi thay vì báo lỗi lệnh, làm build treo ở `Compiling whisper-rs-sys`. Đổi máy (chip có i8mm thật) thì xem lại file này.

### ui/src

| File | Sở hữu | Hàm chính | `invoke` |
|---|---|---|---|
| `main.ts` | Entry: state toàn cục, layout hai cột (hàng đợi + pane phải), màn tải model, chọn từ Ghi âm, đọc bản chép lời, kéo thả | `main` (snapshot + `listen("queue")`, `listen("model-progress")`, `onDragDropEvent`), `render`, `queuePane`, `jobRow`, `select`, `addFiles`, `enqueue`, `reportSkipped`, `openMemos`, `memosPane`, `memoRow`, `refreshMemoControls`, `readerPane`, `firstRun`, `startDownload`, `rightPane` (chọn `reader`/`memos`/`settings`/`note`) | `snapshot`, `enqueue`, `cancel`, `retry`, `remove`, `set_paused`, `list_voice_memos`, `enqueue_voice_memos`, `open_full_disk_settings`, `relaunch`, `read_transcript`, `clean_transcript`, `download_models` |
| `note.ts` | Pane bảng tóm tắt: form tạo, bảng sửa tại chỗ (contentEditable), mục "Cần xác nhận", nghe lại evidence, tự lưu, HTML email | `notePane(id, name, links)` (bên trong: `showForm`, `drawForm`, `showNote`, `draw`, `save`, `play`), `emailHtml` (bảng gửi đi, style inline), `cleanHtml` (whitelist b/i/br/a http), `plainText`, `fromModelText` (`**bold**`, xuống dòng), `handlePaste`, `editable`; hằng `DEFAULT_INTRO`, `SAVE_DELAY_MS` | `ai_settings`, `summarize_job`, `save_note`, `load_note`, `allow_job_audio` |
| `diagram.ts` | Pane Tạo sơ đồ: hiển thị thông tin Gemini + phạm vi gửi trước khi chạy, trạng thái chạy theo từng bản ghi, xem trước có zoom, mục "Cần xác nhận", tạo lại an toàn, xuất PNG và `.excalidraw` | `diagramPane(id, name, hasTranscriptText, links)` | `ai_settings`, `generate_diagram`, `load_diagram`, `save_diagram`, `export_diagram_file` |
| `diagram-render.ts` | Dàn trang flowchart có nhánh/đường quay lại, dựng xem trước SVG, xuất PNG 2x qua Canvas2D (font tiếng Việt đóng gói) và xuất `.excalidraw` v2 chỉnh sửa được | `layoutDiagram`, `renderDiagramSvg`, `renderDiagramPng`, `buildExcalidrawFile` | - |
| `settings.ts` | Pane Cấu hình chia 3 tab (Chép lời, Nhà cung cấp AI, Thuật ngữ), tải lỗi độc lập theo tab, bộ chọn model AI tải lười theo API key | `settingsPane`, `speechSection`, `aiSection`, `providerBlock`, `glossarySection`; export `Provider`, `PROVIDERS`, `PROVIDER_NAME`, `AiView` | `local_models`, `download_local_model`, `select_local_model`, `ai_settings`, `get_glossary`, `save_ai_settings`, `set_api_key`, `delete_api_key`, `list_models`, `test_provider`, `save_glossary`, `import_glossary`, `export_glossary` |
| `dom.ts` | Helper tạo element, mọi text qua `textContent` | `el(tag, props, ...children)` | - |
| `ipc.ts` | Kiểu DTO IPC dùng chung (`Provider`, `AiView`, `SummaryStyleOption`, `MeetingTemplates`) và thanh tiến độ `progressBar` | `PROVIDERS`, `PROVIDER_NAME`, `progressBar` | - |
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
- Kiểm tra UI và lớp Tauri qua app macOS. Hai example `run_job.rs` và `bench.rs` hỗ trợ kiểm tra riêng engine thật.
