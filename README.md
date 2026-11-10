# ASR Studio

Chép lời file ghi âm tiếng Việt ngay trên máy (Rust, Tauri, whisper.cpp). Viết lại từ bộ script Python `ASR_scripts_pack`.

## Build

Cần Rust, CMake, Xcode Command Line Tools, pnpm.

    pnpm --dir ui install
    cd app && ../ui/node_modules/.bin/tauri build

Nếu build đứng mãi ở `Compiling whisper-rs-sys`, xem `.cargo/config.toml`: bước dò CPU của whisper.cpp bị treo trên macOS 27.2 nên kết quả dò được đặt sẵn ở đó.

App ra ở `target/release/bundle/macos/ASR Studio.app`. Lần mở đầu app tải model 1,6 GB về `~/Library/Application Support/asr-studio/models`. Kết quả nằm ở `~/Documents/ASR Studio`.

## Cấu trúc

- `crates/asr-core`: toàn bộ logic. Lớp app chỉ cần biết bốn thứ:
  - `JobRunner`: hàng đợi chép lời. Trả về `Transcript` của việc đã xong (đọc, làm sạch, lưu bảng tóm tắt).
  - `Workspace`: cấu hình AI, API key, danh sách thuật ngữ, và tạo bảng tóm tắt.
  - `models`: kiểm tra, tải và nạp model.
  - `voice_memos`: đọc danh sách của app Ghi âm.
- `app`: lớp mỏng Tauri, mỗi lệnh chỉ chuyển tiếp sang `asr-core`. Thuật ngữ ở `ai/GLOSSARY.md`, mục lục source ở `ai/INDEX.md`.
- `ui`: giao diện TypeScript.

Kiểm tra: `cargo test -p asr-core`, `pnpm --dir ui build`.

## Đo lại engine

`crates/asr-core/examples/bench.rs` so một file WAV 16 kHz với bản chép lời có sẵn. Độ lệch từ không phải tỉ lệ lỗi: chỉ nén AAC một lần cùng engine đã lệch khoảng 34%.
