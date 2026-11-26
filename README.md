# A-Sir

Ứng dụng desktop chép lời tiếng Việt **tại máy (offline)** và tạo biên bản cuộc họp bằng AI, hỗ trợ cả **macOS** và **Windows**.

## Tính năng chính

- **Chép lời tiếng Việt hoàn toàn trên máy:** dùng `whisper.cpp` (tăng tốc Metal trên Apple Silicon / CPU trên Windows), hỗ trợ Whisper Large V3 Turbo, Large V3, PhoWhisper, Qwen3-ASR và tìm/cài thêm model GGML từ Hugging Face.
- **Lọc khoảng lặng (Silero VAD) & hàng đợi thông minh:** chia nhỏ bản ghi dài, tự khôi phục nếu app đóng đột ngột.
- **Nhóm & thư mục bản ghi:** tạo nhóm ngay trên thanh bên, kéo thả hoặc chuyển bản ghi vào nhóm, tự đồng bộ thư mục xuất kết quả trên ổ cứng.
- **Biên bản cuộc họp AI:** kết nối Gemini, OpenAI (ChatGPT) hoặc Claude — tự lọc đúng model tài khoản gọi được, tùy chọn văn phong tiếng Việt tự nhiên (Humanizer) và xuất file **Word (`.docx`) / Excel (`.xlsx`)** theo mẫu chuẩn hoặc mẫu riêng.
- **Từ điển thuật ngữ:** giúp nhận diện chính xác tên riêng, sản phẩm và từ viết tắt.

---

## Cài đặt nhanh (Bản dựng sẵn)

Tải bộ cài mới nhất tại mục **[Releases](../../releases)**:

### macOS (Apple Silicon / macOS 13.3+)
1. Tải file `A-Sir_*_aarch64.dmg` từ trang **Releases**.
2. Mở file `.dmg` và kéo **A-Sir** vào thư mục **Applications**.
3. Lần đầu mở, nếu macOS báo ứng dụng chưa ký chứng chỉ Apple Developer, bấm chuột phải vào **A-Sir** → **Open** → **Open** (hoặc chạy `xattr -cr "/Applications/A-Sir.app"` trong Terminal).

### Windows (Windows 10 / 11 64-bit)
1. Tải file `A-Sir_*_x64-setup.exe` (hoặc `.msi`) từ trang **Releases**.
2. Chạy trình cài đặt và làm theo hướng dẫn.
3. Windows 10/11 thường đã có sẵn **Microsoft Edge WebView2 Runtime**; nếu thiếu, trình cài đặt sẽ tự tải bổ sung.

---

## Cài đặt & Build từ mã nguồn

### Yêu cầu chung
- **Rust** (stable): cài qua [rustup.rs](https://rustup.rs)
- **Node.js 20+** và **pnpm**: `npm install -g pnpm`
- **CMake** (để biên dịch `whisper.cpp`)

### 1. Trên macOS

Cài Xcode Command Line Tools và CMake (nếu chưa có):
```bash
xcode-select --install
brew install cmake
```

Clone repo, cài dependencies và build `.app` / `.dmg`:
```bash
git clone https://github.com/kiettang974/a-sir.git
cd a-sir

# Cài dependencies giao diện
cd ui && pnpm install && cd ..

# Chạy thử chế độ dev
cd app && ../ui/node_modules/.bin/tauri dev

# Build bản cài đặt macOS (.app và .dmg)
cd app && ../ui/node_modules/.bin/tauri build --bundles app,dmg
```

Sản phẩm sau khi build nằm tại:
- `target/release/bundle/macos/A-Sir.app`
- `target/release/bundle/dmg/A-Sir_*.dmg`

Cài nhanh vào `/Applications`:
```bash
rm -rf "/Applications/A-Sir.app"
ditto "target/release/bundle/macos/A-Sir.app" "/Applications/A-Sir.app"
```

### 2. Trên Windows

Cài công cụ biên dịch C/C++ (PowerShell với quyền Administrator):
1. Cài **Visual Studio Build Tools 2022** (chọn workload **Desktop development with C++**).
2. Cài **LLVM** (cho `bindgen`) và **CMake**:
   ```powershell
   winget install LLVM.LLVM Kitware.CMake
   ```
3. Thiết lập biến môi trường `LIBCLANG_PATH` trỏ tới thư mục `bin` của LLVM (mặc định `C:\Program Files\LLVM\bin`).

Clone repo, cài dependencies và build `.exe` / `.msi`:
```powershell
git clone https://github.com/kiettang974/a-sir.git
cd a-sir

# Cài dependencies giao diện
cd ui
pnpm install
cd ..

# Chạy thử chế độ dev
cd app
..\ui\node_modules\.bin\tauri dev

# Build bộ cài Windows (.exe NSIS và .msi)
..\ui\node_modules\.bin\tauri build --bundles nsis,msi
```

Bộ cài sau khi build nằm tại:
- `target\release\bundle\nsis\A-Sir_*_x64-setup.exe`
- `target\release\bundle\msi\A-Sir_*_x64_en-US.msi`

---

## Vị trí lưu dữ liệu

| Dữ liệu | macOS | Windows |
| --- | --- | --- |
| Model chép lời đã tải | `~/Library/Application Support/asr-studio/models/` | `%APPDATA%\asr-studio\models\` |
| Cài đặt, hàng đợi, thuật ngữ | `~/Library/Application Support/vn.vmed.asr-studio/` | `%APPDATA%\vn.vmed.asr-studio\` |
| Bản chép lời & biên bản xuất | `~/Documents/ASR Studio/` | `%USERPROFILE%\Documents\ASR Studio\` |

Bạn có thể mở nhanh thư mục lưu model ngay trong ứng dụng tại **Cài đặt → Giọng nói → Mở thư mục model**.

---

## Phát hành tự động (GitHub Actions)

Workflow [.github/workflows/release.yml](.github/workflows/release.yml) tự động build song song cả bản **macOS (`.dmg`, `.app`)** và **Windows (`.exe`, `.msi`)** mỗi khi đẩy tag phiên bản mới:

```bash
git tag v0.1.0
git push origin v0.1.0
```
