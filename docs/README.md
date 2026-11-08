# NeuralIMK — Tài liệu dự án

Thư mục `docs/` là nguồn sự thật về thiết kế. Code là nguồn sự thật về hành vi;
khi hai bên lệch nhau, sửa tài liệu trong cùng commit với code.

## Danh mục

| File | Nội dung | Nguồn sự thật trong code |
| --- | --- | --- |
| [00-overview.md](00-overview.md) | Mục tiêu, phạm vi, ngoài phạm vi, chỉ số thành công | — |
| [01-architecture.md](01-architecture.md) | Process, luồng, crate, quy tắc tầng | `Cargo.toml`, `arch-lint.toml` |
| [02-keyboard-io.md](02-keyboard-io.md) | IMK, composer, marked text, hotkey, lifecycle | `crates/nimk-engine/src/{composer,hotkey}.rs`, `macos/` |
| [03-context-io.md](03-context-io.md) | `bundle_id`, `caret_rel`, đổi toạ độ | `macos/NeuralIMK/InputController.swift`, `nimk-types::CaretRel` |
| [04-engine-ml.md](04-engine-ml.md) | Bài toán, features, model, online learning, nhãn | `crates/nimk-policy`, `crates/nimk-engine/src/{labeler,trainer}.rs` |
| [05-storage-and-log.md](05-storage-and-log.md) | Trait `Storage`, schema sự kiện, replay, metric | `crates/nimk-storage`, `nimk-types::event`, `crates/nimk-replay` |
| [06-gui.md](06-gui.md) | Overlay, status item, menu IME | `macos/NeuralIMK/Overlay.swift` |
| [07-build-and-install.md](07-build-and-install.md) | Build, đóng gói, cài đặt, debug | `Makefile`, `scripts/` |
| [08-coding-standard.md](08-coding-standard.md) | Quy ước code, contract arch-lint, test | `arch-lint.toml`, `Makefile` |
| [adr/](adr/) | Quyết định kiến trúc, mỗi quyết định một file | — |
| [devlog.md](devlog.md) | Nhật ký phát triển (working memory), mới nhất ở trên | — |

## Chuẩn viết tài liệu

### Định dạng

- Markdown GitHub (GFM). Một file một chủ đề; tên file `NN-kebab-case.md`.
- Văn xuôi tiếng Việt; identifier, tên crate, tên API giữ nguyên tiếng Anh trong `backtick`.
- Dòng đầu là `# Tiêu đề`, ngay sau là khối metadata:

  ```markdown
  > **Trạng thái:** draft | stable · **Cập nhật:** YYYY-MM-DD · **Code:** `crates/...`
  ```

- Câu đầu tiên của file và của mỗi mục nêu kết luận, không mở đầu kiểu "Tài liệu này mô tả...".
- Câu < 25 từ, đoạn ≤ 3 câu. Dùng bảng cho dữ liệu nhiều thuộc tính, danh sách đánh số cho quy trình tuần tự.
- Code mẫu trong fence có ngôn ngữ (`rust`, `swift`, `c`, `toml`, `sh`). Chữ ký API trong tài liệu phải khớp code; nếu chỉ minh hoạ, ghi `// minh hoạ`.

### Sơ đồ ASCII

- Mọi sơ đồ là ASCII trong fence ```` ```text ````, không dùng ảnh hay Mermaid, để đọc được trong terminal và diff được.
- Chỉ dùng ký tự ASCII: `+ - | / \ < > v ^ * .` và chữ. Không dùng ký tự vẽ khung Unicode.
- Rộng tối đa 100 cột. Hộp vẽ bằng `+---+` / `|   |`; mũi tên `-->`, `<--`, `<-->`, `|` + `v` cho chiều dọc.
- Nhãn trên mũi tên đặt ngay trên hoặc cạnh mũi tên, không cắt qua hộp.
- Mỗi sơ đồ có một câu ngay dưới giải thích điều cần rút ra.

Ví dụ:

```text
+-------------+   NSEvent    +--------------+   C ABI    +-------------+
|  App focus  | -----------> | Swift shim   | ---------> | Rust core   |
+-------------+              +--------------+            +-------------+
```

Swift chỉ chuyển tiếp; mọi quyết định nằm trong Rust core.

### ADR

- File `adr/NNNN-kebab-title.md`, đánh số tăng dần, không sửa ADR đã `accepted`; thay bằng ADR mới với `Supersedes: NNNN`.
- Mẫu:

  ```markdown
  # NNNN. Tiêu đề

  > **Trạng thái:** proposed | accepted | superseded by NNNN · **Ngày:** YYYY-MM-DD

  ## Bối cảnh
  ## Quyết định
  ## Hệ quả
  ```

### Devlog

- Bảng `Ngày | Loại | Ghi chú`, mới nhất ở trên. Loại: `quyết định`, `phát hiện`, `việc mở`, `xong`.
- Mỗi milestone hoàn thành ghi một dòng `xong` kèm lệnh đã chạy để kiểm chứng.

### Khi nào cập nhật

- Đổi API công khai, schema sự kiện, quy tắc tầng hay config mặc định → cập nhật file tương ứng trong cùng commit.
- Thêm quy tắc vào `arch-lint.toml` → cập nhật `08-coding-standard.md` và `01-architecture.md`.
- Quyết định có đánh đổi → viết ADR.
