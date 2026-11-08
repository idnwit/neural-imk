# Devlog

> **Trạng thái:** stable · **Cập nhật:** 2026-11-08 · **Code:** —

Working memory của quá trình code: quyết định, phát hiện, việc mở; mới nhất ở trên.

| Ngày | Loại | Ghi chú |
| --- | --- | --- |
| 2026-11-08 | quyết định | Model mặc định của engine đổi sang `LinearAction` (người dùng chốt): độ chính xác ngang `GruAction` (render@commit 0.975), `score` p99 0.5 µs so với khoảng 170 µs; `GruAction` vẫn giữ để so sánh qua replay. |
| 2026-11-08 | xong | Đồng bộ `docs/` 00–09 và ADR 0005/0007/0008/0009 với code hiện tại; kiểm tra mọi sơ đồ `text` là ASCII thuần và ≤ 100 cột bằng script ngoài repo |
| 2026-11-08 | phát hiện | Snapshot nằm ở `<root>/blobs/model/<kind>/latest` (`PlainFsStorage` đặt blob dưới `blobs/`), không phải `<root>/model/<kind>/latest`; docs 05/07 ghi đúng đường dẫn thật |
| 2026-11-08 | việc mở | Chưa kiểm chứng trên máy thật: IME trong Input Sources, đường phím → commit, sửa ngược theo app ([09](09-app-compat.md), kế hoạch test tay), overlay màn hình phụ |
| 2026-11-08 | việc mở | Pretrain `gru_action` và copy snapshot trước lần cài đầu ([07](07-build-and-install.md)); đặt QoS thấp cho trainer thread; `Overlay::Auto` mỗi lần sticky mode đổi có thể ồn |
| 2026-11-08 | xong | `make check` xanh: fmt, clippy, 187 test (185 + 2 doc-test) và arch-lint 0 lỗi; 2 test `#[ignore]` của `nimk-policy` chạy tay ở release |
| 2026-11-08 | xong | Tuning ML: `LinearAction` 200x500x2 render@commit 0.975, replay 97.5%; `GruAction` 200x500x2 (102 s) 0.975, replay online 97.3% / frozen 97.4%, edit accuracy 1.000, `score` p99 0.5 µs và khoảng 170 µs ([04](04-engine-ml.md#kết-quả-đo)) |
| 2026-11-08 | phát hiện | Cập nhật render-only làm trôi thân GRU, học từ mọi xác nhận làm model thiên về ngôn ngữ của từ cuối: `learns_from_confirmations() = false` cho `GruAction`; trọng số theo vị trí phím cho render bị bác bỏ; commit dùng quyết định mới của model (+1 điểm), hysteresis chỉ giữa từ |
| 2026-11-08 | phát hiện | Rò nhãn (label leak): khi `teach`, `shown` là render đích của phím trước nên model chép nó (held-out render khoảng 0.65); scalar `SHOWN_VI` dành riêng luôn 0, snapshot cũ phải pretrain lại ([ADR 0007](adr/0007-neural-action-model.md)) |
| 2026-11-08 | quyết định | Người dùng chốt "flip mode + learn only": app không hỗ trợ `replacementRange` thì chỉ đổi mode và học, không dùng backspace giả `CGEvent`, không cần quyền Accessibility ([ADR 0008](adr/0008-double-shift-retro-correction.md)) |
| 2026-11-08 | phát hiện | Nghiên cứu tương thích app: Chromium chuyển `replacementRange` vào `ImeCommitText` nhưng `selectedRange` cache trễ; Ghostty bỏ qua `replacementRange`; Firefox yếu IMKit; phần còn lại chưa kiểm chứng ([09](09-app-compat.md)) |
| 2026-11-08 | xong | Nháy Shift hai lần + sửa ngược: `HotkeyDetector` (`double_shift_secs` 0.4), `EngineOutput.replace`, `engine/switch.rs`, `ReplaceCompat.swift` (pre/post-check, verdict cache, menu toggle); `tests/retro.rs` 15 test |
| 2026-11-08 | quyết định | Backend NN là `burn` =0.22.1 backend `flex`, default-features tắt (không rayon), `[profile.dev.package."*"] opt-level = 3`; loại candle/tract/ort/tch/dfdx ([ADR 0009](adr/0009-burn-nn-backend.md)) |
| 2026-11-08 | quyết định | Pivot sang neural action model: `Observation` → `ImeModel` → `Action`, bỏ luật cứng và `Policy`/`Learner`/`TinyMlp`/`Baseline`; `LinearAction` và `GruAction`, pretrain behaviour cloning ([ADR 0007](adr/0007-neural-action-model.md) accepted, [ADR 0005](adr/0005-online-supervised-not-exploratory-rl.md) superseded một phần) |
| 2026-11-08 | xong | Đồng bộ C ABI với `Engine`: `NimkOut` mang `replace_delete`/`replace_text`/`overlay`, `nimk_commit_pending` có `timestamp`, `nimk_new` nhận `app_version`; 9 hàm xuất, 18 test |
| 2026-11-08 | xong | Chốt `HotkeyDetector`: modifier khác huỷ nháy, `flagsChanged` không phải hotkey trả `consumed = false`, hotkey và Ctrl+Space trả `consumed = true` ([02](02-keyboard-io.md)) |
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
