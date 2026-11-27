# Sổ luật UI — ASR Studio (`design-decisions`)

Tài liệu này ghi các quyết định UI có nguồn để agent **tra luật thay vì đoán** khi thêm, sửa hoặc review màn hình trong `ui/src/`.

## 1. Tầng 1 — Token theo nghĩa

Mọi màu, bo góc, bóng đổ và font phải đi qua token trong `:root` của `ui/src/style.css`:

| Nhóm | Token | Nghĩa sử dụng |
| --- | --- | --- |
| Nền | `--paper`, `--paper-hover`, `--sheet`, `--card`, `--card-selected`, `--row-alt` | Phân lớp thanh bên, trang tài liệu, thẻ nổi, dòng xen kẽ |
| Chữ | `--ink`, `--ink-soft`, `--ink-faint`, `--on-accent` | Văn bản chính, phụ, siêu dữ liệu, chữ trên nền nhấn |
| Trạng thái & nhấn | `--accent`, `--accent-hover`, `--accent-soft`, `--accent-ring`, `--fail`, `--fail-soft` | Hành động chính, mục đang chọn, lỗi / nguy hiểm |
| Tương tác | `--selection`, `--hover-wash`, `--hover-subtle`, `--line-hover`, `--scroll-thumb`, `--scroll-thumb-hover` | Bôi đen, hover nhẹ, thanh cuộn |
| Đường kẻ & bóng | `--rule`, `--rule-strong`, `--shadow-sm`, `--shadow-menu` | Viền mảnh, viền input, bóng nổi menu |
| Bo góc & font | `--radius-sm`, `--radius-md`, `--radius-lg`, `--ui`, `--read` | Bo góc theo cấp phần tử, font UI & đọc |

## 2. Tầng 2 — Component, variant và slot

| Component | Variant / Class | Slot cho phép | Token chính |
| --- | --- | --- | --- |
| Nút hành động chính | `.btn-primary`, `.add` | Thanh công cụ đầu trang (tối đa 1), chân thanh bên, form chính | `--accent`, `--on-accent`, `--radius-sm` |
| Nút hành động phụ | `.btn-ghost` | Thanh công cụ đầu trang (cạnh nút chính) | `--card`, `--rule-strong`, `--ink` |
| Liên kết hành động | `.link`, `.link.danger-link` | Thanh tiêu đề nhỏ, chân thanh bên, hàng thao tác nội dung | `--accent` / `--fail` |
| Nút chỉ icon | `.icon-btn`, `.icon-btn.danger`, `.icon-btn.active` | Thanh đầu/chân `Queue`, cột xoá bảng `Thuật ngữ` (bắt buộc có `title` + `aria-label`) | `--ink-soft`, `--accent`, `--fail-soft` |
| Menu tràn | `overflowMenu` (`.more-menu`) | Cuối `.reader-actions` cho mọi thao tác phụ ít dùng hơn | `--card`, `--shadow-menu`, `--rule` |
| Mốc thời gian nghe lại | `.time` | Trong `.evidence` của `Note` | `--accent`, `--accent-soft` |
| Thanh tiến độ | `progressBar` (`.bar`) | Trong dòng job đang chạy, thẻ tải model | `--rule`, `--accent` |
| Khung thông báo trạng thái | `.status-inline`, `.status-inline.bad`, `.note-status` | Dưới form/khối cấu hình, cạnh tiêu đề biên bản | `--ink-soft` / `--fail` |

## 3. Tầng 3 — Pattern, màn hình và danh sách đóng

### Danh sách đóng: Trạng thái → Nhãn & màu

| Trạng thái | Nhãn hiển thị | Màu / Token |
| --- | --- | --- |
| `queued` | `Đang chờ` | `--ink-soft` |
| `running` | `<percent>%` (+ ETA) hoặc `Đang tạm dừng ở <percent>%` | `--ink-soft` + `.bar` (`--accent`) |
| `done` | `Đã xong` | `--ink-soft` |
| `failed` | `<message>` (luôn kèm chữ nguyên nhân) | `--fail` |
| `cancelled` | `Đã huỷ` | `--ink-soft` |
| Model `selected` | `Đang dùng` | `--accent` + `--on-accent` |
| Model `ready` | `Sẵn sàng` | `--paper` + `--ink` |
| Model `downloading` | `Đang tải <percent>%` | `--ink-soft` + `.bar` |
| Model `not ready` | `Chưa tải` | `--paper` + `--ink-soft` |

### Danh sách đóng: Màn hình & thứ bậc hành động theo slot

| Màn hình | Slot chính (1 nút `.btn-primary` / `.add`) | Slot phụ (`.btn-ghost` / `.link`) | Menu tràn (`⋯`) |
| --- | --- | --- | --- |
| `Queue` (thanh bên) | `Thêm file` (chân thanh bên) | `Tạm dừng`/`Tiếp tục`, `Xoá hàng chờ`, `Từ app Ghi âm`, `Hướng dẫn nhanh` (`?`), `Cấu hình` | Menu `⋯` từng bản ghi (`Huỷ`, `Thử lại`, `Chép lời lại`, `Xoá`) |
| `Reader` (tab `1. Bản chép` / `Tài liệu kèm`) | `Chép bản chép` | Thanh luồng `recordFlowTabs` (`1. Bản chép · 2. Làm sạch · 3. Tóm tắt · 4. Sơ đồ \| Tài liệu kèm`); click mốc giờ để nghe lại, click chữ để sửa trực tiếp | `Chép lời lại`, `Mở trong Finder` |
| `Clean` (tab `2. Làm sạch`) | `Chạy lại làm sạch` | — | — |
| `Note` (tab `3. Tóm tắt`) | `Tạo lại tóm tắt` | `Chép email`, `Excel`, `Word` | `Mở trong Finder` |
| `Diagram` (tab `4. Sơ đồ`) | `Tạo sơ đồ` (chưa có) / `Xuất PNG` (đã có) | `Xuất .drawio`, dropdown chọn kiểu sơ đồ (16 mẫu có hình minh hoạ), dropdown hiển thị Cần xác nhận (4 chế độ), thanh zoom (`−`/`+`/toàn màn hình) | `Tạo lại sơ đồ`, `Mở tệp đã lưu trong Finder` |
| `Settings` (4 tab) | `Lưu thuật ngữ` (tab Thuật ngữ), `Chọn dùng` (model) | `Tải model`, `Xoá`, `Chọn file mẫu`, `Kiểm tra`, `Chọn model` | — |
| `Voice Memos` | `Chép lời <N> bản ghi` | `Chọn tất cả`/`Bỏ chọn`, ô tìm kiếm | — |

## 4. Tầng 4 — Danh mục luật và bậc thực thi

| Mã luật | Nội dung | Ngày chốt | Người chốt | Phạm vi | Trạng thái | Bậc thực thi & chỗ thi hành |
| --- | --- | --- | --- | --- | --- | --- |
| `UI-01` | Không viết tay màu hex hoặc `rgb(...)` ngoài khối `:root` của `ui/src/style.css`; mọi màu đi qua token theo nghĩa. | 2026-11-26 | kietta | `ui/src/style.css` | Còn hiệu lực | Bậc 5 — `ui/scripts/lint-ui-rules.mjs` (chạy trong `pnpm --dir ui build`) |
| `UI-02` | Không viết tay `style: "..."` inline trong `ui/src/*.ts`; dùng class CSS từ `style.css` (ngoại lệ duy nhất: `fill.style.width` động của `progressBar`). | 2026-11-26 | kietta | `ui/src/*.ts` | Còn hiệu lực | Bậc 5 — `ui/scripts/lint-ui-rules.mjs` (chạy trong `pnpm --dir ui build`) |
| `UI-03` | Mọi luật trong sổ luật phải ghi đủ ngày chốt (`YYYY-MM-DD`), người chốt và trạng thái hiệu lực. | 2026-11-26 | kietta | `AI/rules/DESIGN-DECISIONS.md` | Còn hiệu lực | Bậc 5 — `ui/scripts/lint-ui-rules.mjs` |
| `UI-04` | Mỗi nhóm hành động trong cùng một slot có tối đa 1 nút chính (`.btn-primary` / `.add`); hành động phụ đặt cạnh bằng `.btn-ghost` hoặc gom vào `overflowMenu` (`⋯`). | 2026-11-26 | kietta | `ui/src/main.ts`, `ui/src/note.ts`, `ui/src/settings.ts` | Còn hiệu lực | Bậc 4 — `overflowMenu` trong `ui/src/ipc.ts` + review |
| `UI-05` | Mọi trạng thái luôn có chữ giải thích (không chỉ dùng màu); lỗi phải nêu rõ nguyên nhân. | 2026-11-26 | kietta | Toàn bộ `ui/src/` | Còn hiệu lực | Bậc 3 — Sổ luật & checklist review |
| `UI-06` | Không dùng viền trái màu dày (`border-left` / `inset 3px 0`) trên thẻ hoặc dòng danh sách để báo chọn; dùng nền thẻ + viền mảnh đồng bộ hệ màu. | 2026-11-26 | kietta | `ui/src/style.css` | Còn hiệu lực | Bậc 5 — `impeccable detect` |
| `UI-07` | Dùng một màu nhấn duy nhất (`--accent`, xanh lam đậm Cobalt) xuyên suốt toàn app trên hệ trung tính Slate lạnh đồng nhất; không trộn xám ấm/lạnh. | 2026-11-26 | kietta | `ui/src/style.css`, `DESIGN.md` | Còn hiệu lực | Bậc 3 — `DESIGN.md` & review |
| `UI-08` | Số liệu, mốc giờ, thời lượng và STT luôn bật `font-variant-numeric: tabular-nums`; đoạn đọc bản chép giữ độ rộng tối ưu `66ch`. | 2026-11-26 | kietta | `ui/src/style.css` | Còn hiệu lực | Bậc 3 — `ui/src/style.css` |
| `UI-09` | Mọi màn hình chi tiết bản ghi (`Bản chép lời`, `Ngữ cảnh`, `Bản làm sạch`, `Tóm tắt`, `Sơ đồ`) dùng chung thanh luồng `recordFlowTabs` ngay dưới `.reader-head` (tiêu đề dòng trên, thanh nút `.reader-actions` dòng dưới). | 2026-11-27 | kietta | `ui/src/ipc.ts`, `ui/src/main.ts`, `ui/src/note.ts`, `ui/src/diagram.ts`, `ui/src/context.ts` | Còn hiệu lực | Bậc 4 — `recordFlowTabs` trong `ui/src/ipc.ts` |
| `UI-10` | Nút có chú thích hover dùng CSS tooltip tức thì (`data-tip` hiện sau `60ms`, không dùng `title` gốc bị trễ trên macOS) và luôn giữ `aria-label`. | 2026-11-27 | kietta | `ui/src/dom.ts`, `ui/src/style.css` | Còn hiệu lực | Bậc 4 — `el()` trong `ui/src/dom.ts` |
| `UI-11` | Không hiển thị đoạn `.hint` giải thích dư thừa cho tính năng/nút đã tự rõ nghĩa và không render ô nhập rỗng không nhãn (chỉ hiện khi có dữ liệu hoặc người dùng chủ động bật). | 2026-11-27 | kietta | `ui/src/note.ts`, `ui/src/settings.ts`, `ui/src/context.ts` | Còn hiệu lực | Bậc 3 — Sổ luật & checklist review |
