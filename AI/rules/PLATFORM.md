# Quy tắc nghiệp vụ và dữ liệu

## Chép lời và lưu kết quả

- Giải mã và nhận dạng âm thanh tại máy. `JobRunner` xử lý mỗi lần một việc và lưu các khúc đã hoàn thành vào checkpoint để chạy tiếp.
- Dùng quy tắc đặt tên của `output` và `transcript` để giữ các tệp chép lời đã có khi xuất thêm bản mới.
- Khi thay model chép lời, kiểm tra hành vi resume. Checkpoint hiện nhận diện file nguồn bằng kích thước và thời điểm sửa. Một job tiếp tục sau khi đổi model cần giữ cùng model từ đầu hoặc chạy lại các khúc đã chép bằng model khác.
- Mốc giờ trong bản chép lời và bảng biên bản phục vụ nghe lại. Cấu hình mặc định lấy mốc đoạn từ vùng tiếng nói do VAD phát hiện.

## Tóm tắt và tích hợp AI

- Khi người dùng yêu cầu tóm tắt, gửi bản chép lời và thuật ngữ đến đúng nhà cung cấp được chọn. Dùng dữ liệu giả lập khi kiểm thử API, UI hoặc phát lại fixture.
- API key do `Workspace` quản lý qua `ApiKeys`. Payload cấu hình lên UI chỉ chứa gợi ý đã che key.
- Glossary được đưa vào prompt tóm tắt. Bản chép lời gốc là đầu vào riêng cho thao tác làm sạch và đối chiếu.
- Bảng sao chép để gửi đi chứa các mục đã chốt. Mục cần xác nhận ở lại trong bản nháp để người dùng xử lý.
- Cấp quyền phát audio cho từng file nguồn của job qua `allow_job_audio` và giữ quyền plugin trong `app/capabilities/default.json`.

Xem [bản đồ mã nguồn](../docs/STRUCTURE.md) để định vị các mô-đun và lệnh.
