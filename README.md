# A-Sir

A-Sir chép lời tiếng Việt ngay trên máy bạn. Từ bản chép lời, app nhờ AI tạo biên bản cuộc họp và xuất ra Word hoặc Excel. Chạy trên macOS và Windows.

## Cài đặt

Vào trang [Releases](../../releases) và tải bản phù hợp:

| Hệ điều hành | File cần tải |
| --- | --- |
| macOS 13.3 trở lên | `A-Sir_*.dmg` |
| Windows 10/11 (64-bit) | `A-Sir_*_x64-setup.exe` hoặc `.msi` |

**macOS:** mở file `.dmg`, kéo A-Sir vào thư mục Applications. Nếu macOS chặn vì app chưa ký, bấm chuột phải vào app, chọn Open. Hoặc chạy lệnh:

```bash
xattr -cr "/Applications/A-Sir.app"
```

**Windows:** chạy file cài đặt rồi làm theo hướng dẫn.

Lần đầu mở, vào **Cấu hình → Chép lời** để tải model. Muốn tạo biên bản thì thêm API key của Gemini, ChatGPT hoặc Claude ở tab **Tài khoản AI**. Từ bản sau, app tự báo khi có phiên bản mới ở tab **Cập nhật**.

## Tự build từ mã nguồn

Bạn cần Rust, Node.js 20 trở lên, pnpm và CMake.

```bash
git clone https://github.com/kiettang974/a-sir.git
cd a-sir/ui && pnpm install && cd ../app
../ui/node_modules/.bin/tauri build
```

- **macOS:** cài Xcode Command Line Tools trước (`xcode-select --install`).
- **Windows:** cài Visual Studio Build Tools (workload C++), LLVM và CMake. Đặt `LIBCLANG_PATH` trỏ tới thư mục `bin` của LLVM.

Bộ cài nằm trong `target/release/bundle/`. Chạy thử khi đang phát triển: `../ui/node_modules/.bin/tauri dev`.

## Dữ liệu lưu ở đâu

| Dữ liệu | macOS | Windows |
| --- | --- | --- |
| Model chép lời | `~/Library/Application Support/asr-studio/models/` | `%APPDATA%\asr-studio\models\` |
| Cài đặt và hàng đợi | `~/Library/Application Support/com.asir.app/` | `%APPDATA%\com.asir.app\` |
| Bản chép lời và biên bản | `~/Documents/ASR Studio/` | `%USERPROFILE%\Documents\ASR Studio\` |

Gỡ app không xóa các thư mục này. Muốn dọn sạch, hãy xóa chúng bằng tay.
