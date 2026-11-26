# Bản xem trước giao diện trên trình duyệt
Thư mục này giả lập các API của Tauri (`@tauri-apps/...`) để xem và chụp màn hình giao diện trong trình duyệt thường, không cần chạy app.
Chạy: `pnpm --dir ui preview:mock` rồi mở http://localhost:5180/ (cấu hình ở `ui/vite.preview.config.ts`).
Chọn màn hình qua URL: `?state=first-run`, `?state=empty`, `?memos=denied|failed|empty`, `?keys=none`, `?note=saved`, `?paused=1`, `?tick=0`, `?open=reader|note|settings|memos`.
Toàn bộ dữ liệu mẫu trong `fixtures.ts` là nội dung bịa ra, không lấy từ bản ghi thật.
