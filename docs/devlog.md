# Devlog

> **Trạng thái:** stable · **Cập nhật:** 2026-11-08 · **Code:** —

Working memory của quá trình code: quyết định, phát hiện, việc mở; mới nhất ở trên.

| Ngày | Loại | Ghi chú |
| --- | --- | --- |
| 2026-11-08 | phát hiện | arch-lint 0.11: `no-panic-in-lib` (AL011) không bật được qua `[rules]`; `make arch` chạy pass thứ hai với `--rules no-panic-in-lib` |
| 2026-11-08 | phát hiện | arch-lint 0.11: `deny-attr` không bắt dạng edition 2024 `#[unsafe(no_mangle)]`; lint workspace `unsafe_code = "deny"` bắt thay |
| 2026-11-08 | phát hiện | arch-lint 0.11: `[[independent]]` và `[[deny-scope-dep]]` không resolve đường dẫn `crate::` trong layout này (đã probe); độc lập module engine khoá bằng `restrict-use` trên text `crate::<mod>` |
| 2026-11-08 | quyết định | Pretrain bằng dữ liệu synthetic qua crate `nimk-synth` ([ADR 0006](adr/0006-synthetic-pretraining.md)) |
| 2026-11-08 | quyết định | Hotkey `after_commit` chỉ đổi mode cho từ kế tiếp, không viết lại text đã commit; ngoài phạm vi PoC dù review đã nêu ([ADR 0003](adr/0003-shift-tap-hotkey.md)) |
| 2026-11-08 | quyết định | Review kiến trúc: `Engine::new` không bao giờ lỗi (fallback model mới / null sink); snapshot policy qua mpsc thay `ArcSwap`; `nimk-storage` thành leaf thật; schema thêm `seq`, `session_start`, `Target` |
| 2026-11-08 | việc mở | Đồng bộ hợp đồng C ABI với `nimk-engine`: `NimkOut` cần mang `Overlay` (auto/manual) thay cho `show_overlay: bool`; `NimkCtx` cần `field_id` và `timestamp`; `nimk_commit_pending` cần `timestamp`; `nimk_new` cần `SessionInfo` |
| 2026-11-08 | việc mở | Chốt hành vi `HotkeyDetector` khi modifier khác đổi lúc giữ Shift, và `consumed` cho `flagsChanged`/Ctrl+Space ([02](02-keyboard-io.md)) |
| 2026-11-08 | xong | Viết `docs/` 00–08, ADR 0001–0006 theo chuẩn `docs/README.md`, khớp contract sau review kiến trúc |
| 2026-11-08 | phát hiện | arch-lint không quy `use other_crate::…` về scope; ràng buộc giữa crate dùng `restrict-use` deny theo đường dẫn crate, kết hợp dependency trong `Cargo.toml` |
| 2026-11-08 | xong | M0: workspace skeleton build được, contract `arch-lint.toml` (arch-lint 0.11) đã kiểm chứng: probe `use nimk_engine` trong `nimk-policy` bị chặn với lỗi `ALD001` (`make arch`) |
| 2026-11-08 | quyết định | Học có giám sát online, không exploration (ε = 0), hysteresis 0.2 ([ADR 0005](adr/0005-online-supervised-not-exploratory-rl.md)) |
| 2026-11-08 | quyết định | Storage đi qua trait để sau này thêm `EncryptedStorage` dạng decorator ([ADR 0004](adr/0004-storage-behind-trait.md)) |
| 2026-11-08 | quyết định | Hotkey mặc định = gõ nhanh Shift trái, dự phòng Ctrl+Space; không dùng ⌘+Space ([ADR 0003](adr/0003-shift-tap-hotkey.md)) |
| 2026-11-08 | quyết định | IMK + Swift shim mỏng, Rust core link tĩnh; chấp nhận gạch chân marked text ([ADR 0002](adr/0002-swift-shim-rust-core.md)) |
| 2026-11-08 | quyết định | Một input source IMK duy nhất, VI/EN là chế độ nội bộ ([ADR 0001](adr/0001-single-imk-input-source.md)) |
