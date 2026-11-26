# ASR Studio

Ứng dụng macOS chép lời tiếng Việt tại máy bằng Rust, `whisper.cpp` qua `whisper-rs` và Metal. Giao diện hiện tại dùng Tauri 2, TypeScript thuần và Vite. Model chép lời hiện tại là Whisper Large V3 Turbo với Silero VAD.

## Bắt đầu

1. Đọc [mục lục AI](AI/README.md) và [bản đồ mã nguồn](AI/docs/STRUCTURE.md). Khi có ticket, đọc spec và tiêu chí nghiệm thu liên quan trong `.scratch/<feature>/`.
2. Khi repo có `.codegraph/`, dùng `codegraph_explore` hoặc `codegraph explore` trước khi dùng grep, find hoặc đọc source để định vị code. Khi chưa có index, dùng công cụ tìm kiếm của repo.
3. Kiểm chứng thay đổi theo [quy trình kiểm tra](AI/rules/WORKFLOW.md). Cập nhật bản đồ khi thêm, đổi tên hoặc xoá mô-đun hay điểm vào được mô tả tại đó.

## Ranh giới và dữ liệu

- `crates/asr-core/` sở hữu nghiệp vụ chép lời, hàng đợi, model và biên bản. `app/` nối nghiệp vụ với Tauri IPC và quyền macOS. `ui/` sở hữu giao diện và trạng thái hiển thị. Khi sửa payload IPC, đối chiếu DTO Rust và kiểu TypeScript.
- Xử lý audio tại máy. Khi người dùng bấm tạo tóm tắt, app gửi bản chép lời và thuật ngữ đến đúng nhà cung cấp đã chọn. Dùng dữ liệu giả lập cho test, fixture và ảnh chụp. Xem [quy tắc nghiệp vụ và dữ liệu](AI/rules/PLATFORM.md) khi sửa resume, API key, phát audio hoặc tệp kết quả.
- Khi đánh giá model chép lời, dùng cùng audio, cấu hình VAD và máy. Dùng bản chép đã đối chiếu với audio để tính WER. Xem [quy trình benchmark](AI/rules/WORKFLOW.md#khi-đánh-giá-model-chép-lời).

## Tài liệu theo công việc

- **Issue và triage.** Lưu issue Markdown trong `.scratch/<feature>/` theo [quy ước issue cục bộ](AI/docs/issue-tracker.md) và [nhãn triage](AI/docs/triage-labels.md).
- **Thiết lập model.** Đọc [spec Cấu hình và model](.scratch/settings-and-models/spec.md) khi sửa cài đặt, tải model cục bộ hoặc bộ chọn model AI.
- **Biên bản.** Đọc [spec tóm tắt và Excel](.scratch/meeting-deliverables/spec.md) khi sửa thao tác tạo tóm tắt hoặc xuất Excel. Dùng BA-Kit (`ba-kit-meet`) cho nội dung cuộc họp và tiêu chí đã ghi trong ticket.
- **Kiến trúc.** Dùng [thuật ngữ và ADR](AI/docs/domain.md) khi đổi ranh giới mô-đun. [Rust + egui](Speech-to-Text-Rust-egui-Architecture.md) là đề xuất V2 cần POC và benchmark. Khi sửa app hiện tại, lấy Cargo workspace và mã nguồn Tauri làm căn cứ.
