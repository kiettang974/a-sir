@AGENTS.md

## Làm việc với Claude Code

- Khi cần viết yêu cầu hoặc tiêu chí nghiệm thu, dùng `ba-requirements` hoặc `ba-kit-ac` nếu có trong phiên.
- Khi thử giao diện, dùng `pnpm --dir ui preview:mock` nếu checkout có [mock Tauri](ui/mock/README.md). Dùng app macOS để kiểm chứng IPC, quyền hệ điều hành và xử lý audio thật.
- Khi báo kết quả, ghi lệnh đã chạy và phần còn cần kiểm chứng theo [quy trình kiểm tra](AI/rules/WORKFLOW.md).
