# Làm việc và kiểm tra ASR Studio

1. Mở [bản đồ mã nguồn](../docs/STRUCTURE.md) và đối chiếu đoạn code liên quan. Đọc [quy tắc dữ liệu](PLATFORM.md) khi sửa luồng âm thanh, bản chép lời, cấu hình AI hoặc tệp đầu ra.
2. Sửa tại lớp sở hữu hành vi. Đặt nghiệp vụ trong `asr-core`, IPC và quyền macOS trong `app`, trạng thái hiển thị trong `ui`. Nếu thay payload Tauri, kiểm tra kiểu TypeScript và DTO Rust cùng lúc.
3. Chạy kiểm tra theo vùng thay đổi và ghi rõ kết quả thực tế. Cập nhật bản đồ khi mô-đun hoặc điểm vào thay đổi.

| Vùng thay đổi | Kiểm tra |
| --- | --- |
| Hàng đợi, tệp đầu ra, model, AI, glossary | `cargo test -p asr-core`. Dùng fake `Transcriber` hoặc `LlmClient` trong các test tương ứng. |
| Lệnh Tauri, quyền, phát audio | `cargo check -p asr-studio` và thử luồng tương ứng trong app macOS. |
| Giao diện TypeScript/CSS | `pnpm --dir ui build`. Dùng `pnpm --dir ui preview:mock` nếu checkout có mock. |
| Chỉ sửa tài liệu | Kiểm tra liên kết, đường dẫn đúng chữ hoa/thường và lệnh được ghi có trong repo. |

## Khi đánh giá model chép lời

- Đo trước bằng `crates/asr-core/examples/bench.rs` trên cùng máy, cùng audio, cùng cấu hình VAD và gắn kết quả với đúng tệp model.
- Dùng bản chép đã đối chiếu với audio nếu muốn tính WER. Benchmark sẵn có báo độ lệch từ với bản do engine cũ tạo và dùng cách chuẩn hoá riêng. Khi cần so với VietAudio, dùng cùng định nghĩa O-WER và N-WER với bảng xếp hạng.
- Chạy ít nhất một file họp dài qua `examples/run_job.rs` hoặc app. Kiểm tra thời gian xử lý, tên riêng, thuật ngữ, số liệu, file TXT/SRT và khả năng nghe lại từ mốc trích dẫn.

## Quy ước đánh version & ghi bản mô tả cập nhật

Cập nhật đồng bộ trong `app/tauri.conf.json`, `app/Cargo.toml` và `CHANGELOG.md` trước khi push nhánh `release` (hoặc tạo bản phát hành mới):

- **Bản cập nhật nhỏ** (sửa lỗi, tinh chỉnh UI/UX): tăng phần patch `0.Y.X` (`X` từ `1` đến tối đa `10`; vượt `10` thì chuyển lên `0.(Y+1).0`).
- **Bản cập nhật lớn** (tính năng mới đáng kể): tăng phần minor `0.Y.0` (`Y` từ `1` đến tối đa `10`; vượt `10` thì chuyển lên `1.0.0`).
- **Bản mô tả cập nhật (`CHANGELOG.md`):** Viết theo chuẩn skill `vietnamese-tech-writing` / humanizer — tiếng Việt tự nhiên, nêu rõ người dùng nhận được gì mới và lỗi gì đã được sửa (`Đã thêm…`, `Đã sửa…`), không liệt kê tên file/code nội bộ. Workflow `.github/workflows/release.yml` sẽ tự động trích đúng mục `## v<version>` đưa lên GitHub Release.
