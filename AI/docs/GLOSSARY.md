# Thuật ngữ

Các từ dùng trong code và khi bàn về ASR Studio.

**Job (việc)**: một lần chép lời một file ghi âm. Trạng thái: Queued, Running, Done, Failed, Cancelled. Queued và Running là việc *đang hoạt động*.

**Queue (hàng đợi)**: các Job cùng cờ `paused` và `busy`. `busy` nghĩa là còn việc đang hoạt động và không tạm dừng; app giữ máy thức khi `busy`. `JobRunner` xác định trạng thái `busy`.

**Rejected**: lý do một file không vào được hàng đợi: đang có việc hoạt động cho đúng file đó, hoặc định dạng không đọc được. File đã xong, lỗi hay đã huỷ thì thêm lại được.

**Chunk / Checkpoint**: đoạn khoảng 15 phút cắt ở chỗ im lặng; Checkpoint lưu các đoạn đã xong để chạy tiếp sau khi app tắt.

**Transcript (bản chép lời)**: file `.txt` của một Job đã xong, mỗi dòng `[HH:MM:SS --> HH:MM:SS] lời`. Module `transcript` đọc định dạng này và đặt tên các file đi kèm.

**Cleaned transcript (bản đã làm sạch)**: `<tên>_fixed.txt`, bỏ dòng quảng cáo kênh mà Whisper nghe nhầm và gộp từ lặp. Không bao giờ ghi đè: bản giống hệt thì dùng lại, bản khác (có thể đã sửa tay) thì giữ, bản mới thành `_fixed (2).txt`.

**Draft (bản nháp bảng tóm tắt)**: kết quả AI trả về: `rows` (đã chốt) và `pending` (cần xác nhận). Mỗi row có `evidence` (câu trích kèm mốc giờ để nghe lại) và `uncertain` (từ có thể nghe sai).

**Note (meeting note)**: Draft mà người dùng đã sửa, thêm lời chào và câu mở đầu; lưu thành `<tên>_note.json` và `<tên>_note.html`. Hình dạng của Note do giao diện quyết định.

**Pending (cần xác nhận)**: mục đã bàn nhưng chưa rõ đã chốt. Không bao giờ vào bảng gửi đi.

**Workspace**: cấu hình của người dùng trong thư mục dữ liệu của app: nhà cung cấp AI và model, API key, danh sách thuật ngữ.

**Provider (nhà cung cấp AI)** là Claude, ChatGPT hoặc Gemini. Khi tạo bảng tóm tắt, app gửi bản chép lời và danh sách thuật ngữ tới nhà cung cấp được chọn.

**Glossary / Term (thuật ngữ)**: từ viết đúng, cách hay bị nghe nhầm, giải thích. Chỉ dùng khi tạo bảng tóm tắt, không sửa Transcript.

**Model chép lời** là Whisper Large V3 Turbo trong cấu hình hiện tại. **Silero VAD** phát hiện các đoạn có tiếng nói. Module `models` tải và kiểm tra SHA-256 của hai tệp trước khi nạp engine. **Model tóm tắt** là model của nhà cung cấp AI trong `Workspace`.

**Recording (ghi âm)**: một mục trong app Ghi âm của macOS; `on_disk = false` nghĩa là còn trên iCloud.
