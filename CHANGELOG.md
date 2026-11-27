# Lịch sử cập nhật A-Sir

## v0.4.9 — 2026-11-27

### Tính năng mới
- **Đã thêm màn hình chào 3 bước và tour hướng dẫn nhanh cho người mới:** Lần đầu mở ứng dụng, A-Sir tự giới thiệu 5 bước chính từ nạp file ghi âm đến xuất biên bản và vẽ sơ đồ. Bạn có thể mở lại hướng dẫn bất cứ lúc nào bằng nút `?` ở góc dưới thanh bên.
- **Đã thêm thanh luồng 4 bước trên cùng một màn hình:** Chuyển qua lại 1 chạm giữa **1. Bản chép → 2. Làm sạch → 3. Tóm tắt → 4. Sơ đồ** cùng mục **Tài liệu kèm** mà không phải quay ngược về bản chép lời.
- **Đã thêm nghe lại theo mốc giờ và sửa chữ trực tiếp trong bản chép lời:** Bấm vào mốc thời gian (ví dụ `00:54`) để tua nghe lại đúng đoạn đó, hoặc bấm thẳng vào đoạn chữ để sửa và tự lưu mà vẫn giữ nguyên mốc giờ.
- **Đã mở rộng bộ vẽ sơ đồ lên 16 mẫu hình có hình minh hoạ và xuất `.drawio`:** Bạn có thể đổi nhanh kiểu sơ đồ (Swimlane, Sequence, Vòng đời trạng thái, Trước – Sau, Phễu, Ma trận 2×2, Biểu đồ KPI, Vòng lặp nhân quả, Bản đồ khái niệm…), chọn 4 cách hiển thị mục **Cần xác nhận**, và xuất file `.drawio` để chỉnh sửa tiếp trong draw.io.

### Cải thiện & sửa lỗi
- **Đã đưa nút Tạo lại tóm tắt lên làm nút chính** trên màn hình Tóm tắt, đặt **Chép email** cạnh **Excel** và **Word** cho gọn thao tác.
- **Đã tăng chiều cao khung xem sơ đồ lên 70% màn hình**, tự tô vàng các nút có dấu `?`, thu gọn mục **Cần xác nhận** dạng mở/đóng (xem được cả khi phóng to), và bỏ nút **Chỉ lọc tại máy** thừa.
- **Đã sửa lỗi chữ dài bị tràn khỏi cột trong sơ đồ Swimlane** và bổ sung tự xuống dòng cho toàn bộ hộp/hình thoi.
- **Đã sửa lỗi tooltip hiện chậm trên macOS** (chuyển sang tooltip hiện tức thì) và khắc phục tình trạng tooltip che mất menu khi đang mở.
- **Đã sửa lỗi ô nhập trống thừa** phía trên bảng biên bản và dọn các đoạn chữ giải thích dài dòng trong Cài đặt.

## v0.4.8 — 2026-11-27

- **Đã chuyển toàn bộ engine vẽ sơ đồ sang Rust thuần** với nét vẽ tay Excalidraw, 188 biểu tượng hạ tầng dựng sẵn và bộ kiểm tra bố cục tự động.
- **Đã thêm cửa sổ xem sơ đồ phóng to** kèm thanh thu phóng gắn gọn trên khung xem.
