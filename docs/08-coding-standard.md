# Coding standard

> **Trạng thái:** draft · **Cập nhật:** 2026-11-08 · **Code:** `Cargo.toml`, `clippy.toml`, `arch-lint.toml`, `Makefile`

`make check` (fmt + clippy + test + arch-lint) phải xanh trước mỗi commit.
Mọi quy tắc dưới đây được máy kiểm; quy tắc chưa kiểm được ghi rõ là quy ước.

## Cổng kiểm tra

| Bước | Lệnh (`Makefile`) | Chặn |
| --- | --- | --- |
| `fmt` | `cargo fmt --all -- --check` | Code chưa format |
| `lint` | `cargo clippy --workspace --all-targets -- -D warnings` | Mọi warning của rustc và clippy, kể cả lint workspace |
| `test` | `cargo test --workspace` | Test đỏ |
| `arch` | `cargo arch-lint check` rồi `cargo arch-lint check --rules no-panic-in-lib` | Vi phạm contract kiến trúc |

## Lint workspace

Mọi crate khai báo `[lints] workspace = true`; lint đặt ở `Cargo.toml` gốc và `clippy.toml`.

| Lint | Mức | Nguồn | Ý nghĩa |
| --- | --- | --- | --- |
| `unsafe_code` | deny | `[workspace.lints.rust]` | Cấm `unsafe` ở mọi crate; `nimk-ffi` phải tự `allow` có chủ đích. Bắt cả `#[unsafe(no_mangle)]` |
| `missing_docs` | warn (→ lỗi qua `-D warnings`) | `[workspace.lints.rust]` | Mọi item công khai phải có doc |
| `clippy::unwrap_used` | deny | `[workspace.lints.clippy]` | Không `unwrap()` |
| `clippy::expect_used` | deny | `[workspace.lints.clippy]` | Không `expect()` |
| `clippy::print_stdout` | deny | `[workspace.lints.clippy]` | Không `println!`; `nimk-replay` được `allow` vì in report |
| `allow-unwrap-in-tests`, `allow-expect-in-tests` | true | `clippy.toml` | Test được `unwrap`/`expect` |

Mỗi crate trừ `nimk-ffi` còn có `#![forbid(unsafe_code)]` ở `lib.rs`/`main.rs`.

## Contract arch-lint

`arch-lint.toml` (arch-lint 0.11, `preset = "recommended"`, `fail_on = "warning"`) khoá tầng và hot path theo đường dẫn `use`.

### Scope

| Scope | Đường dẫn |
| --- | --- |
| `types`, `storage`, `telex`, `policy`, `engine`, `ffi`, `replay`, `synth` | `crates/nimk-<tên>/src/**` |
| `pure` | `nimk-types`, `nimk-telex`, `nimk-policy` |
| `eng-composer`, `eng-hotkey`, `eng-labeler`, `eng-trainer`, `eng-logger` | Từng file `crates/nimk-engine/src/<module>.rs` |
| `engine-no-threads` | `crates/nimk-engine/src/**` trừ `trainer.rs`, `logger.rs` |
| `non-ffi` | `crates/*/src/**` trừ `crates/nimk-ffi/src/**` |

### Quy tắc

| Quy tắc | Loại | Scope | Cấm | Bảo vệ điều gì |
| --- | --- | --- | --- | --- |
| `types-is-bottom` | `restrict-use` | `types` | `nimk_storage`, `nimk_telex`, `nimk_policy`, `nimk_engine`, `nimk_ffi` | `nimk-types` là tầng đáy |
| `leaf-storage` | `restrict-use` | `storage` | `nimk_telex`, `nimk_policy`, `nimk_engine`, `nimk_ffi` | Storage không biết domain |
| `leaf-telex` | `restrict-use` | `telex` | `nimk_storage`, `nimk_policy`, `nimk_engine`, `nimk_ffi` | Telex là hàm thuần |
| `leaf-policy` | `restrict-use` | `policy` | `nimk_storage`, `nimk_telex`, `nimk_engine`, `nimk_ffi`, `std::fs`, `std::thread` | Policy chỉ map `Features` → `p(VI)` |
| `engine-not-up` | `restrict-use` | `engine` | `nimk_ffi`, `std::fs` | Engine không biết FFI, ghi file qua `Storage` |
| `ffi-thin` | `restrict-use` | `ffi` | `nimk_policy`, `nimk_telex`, `std::fs` | FFI chỉ adapt `Engine` sang C |
| `replay-not-ffi` | `restrict-use` | `replay` | `nimk_ffi` | Replay gọi `Engine` trực tiếp |
| `synth-not-ffi` | `restrict-use` | `synth` | `nimk_ffi` | Synth gọi `Engine` trực tiếp |
| `pure-no-side-effects` | `restrict-use` | `pure` | `std::fs`, `std::io`, `std::net`, `std::thread`, `std::sync` (trừ `Arc`, `atomic`), `std::time`, `std::env`, `parking_lot` | Crate thuần: không I/O, thread, lock, env, wall clock |
| `engine-event-clock` | `restrict-use` | `engine` | `std::time`, `std::env` | Đồng hồ duy nhất là timestamp của event (replay tất định) |
| `hot-path-no-io` | `restrict-use` | `engine-no-threads` | `std::fs`, `std::net`, `std::thread`, `Mutex`, `RwLock`, `Condvar` | Đường phím không I/O, lock, thread |
| `eng-composer-independent` | `restrict-use` | `eng-composer` | `crate::`/`super::` `hotkey`, `labeler`, `trainer`, `logger`, `engine` | Module engine độc lập |
| `eng-hotkey-independent` | `restrict-use` | `eng-hotkey` | Như trên, trừ chính nó | Như trên |
| `eng-labeler-independent` | `restrict-use` | `eng-labeler` | Như trên, trừ chính nó | Như trên |
| `eng-trainer-independent` | `restrict-use` | `eng-trainer` | Như trên, trừ chính nó | Như trên |
| `eng-logger-independent` | `restrict-use` | `eng-logger` | Như trên, trừ chính nó | Như trên |
| `no-c-exports-outside-ffi` | `deny-attr` | `non-ffi` | `#[no_mangle]`, `#[export_name]` | Chỉ `nimk-ffi` export symbol C |
| `no-unwrap-expect` | built-in | toàn repo | `unwrap`/`expect` ngoài test | Trùng lint clippy, chạy cả khi không build |
| `no-panic-in-lib` | built-in (AL011) | toàn repo | `panic!` và tương tự trong lib, ngoài test | Chạy ở pass thứ hai của `make arch` |

Cấm theo pattern `<path>` và `<path>::**`; ràng buộc giữa crate còn được `Cargo.toml` khoá lần nữa vì crate thiếu dependency không thể `use`.

### Giới hạn đã biết của arch-lint 0.11

| Giới hạn | Cách xử lý |
| --- | --- |
| Không quy `use other_crate::…` về scope | Tầng giữa crate dùng `restrict-use` deny theo đường dẫn crate |
| `[[independent]]`, `[[deny-scope-dep]]` không resolve `crate::` trong layout này | Độc lập module engine dùng `restrict-use` trên text `crate::<mod>` và `super::<mod>` |
| `deny-attr` không bắt dạng edition 2024 `#[unsafe(no_mangle)]` | Lint rustc `unsafe_code = "deny"` bắt thay |
| `no-panic-in-lib` không bật được qua `[rules]` | `make arch` chạy pass thứ hai với `--rules no-panic-in-lib` |

### Thêm quy tắc

1. Thêm scope/quy tắc vào `arch-lint.toml`, kèm `message` nói lý do.
2. Viết probe vi phạm, chạy `make arch`, xác nhận bị chặn, rồi xoá probe.
3. Cập nhật bảng trên và [01-architecture.md](01-architecture.md) trong cùng commit.

## Quy ước code

| Quy ước | Kiểm bởi |
| --- | --- |
| Swift shim không có logic quyết định; đổi hành vi làm trong Rust, kèm test | Review |
| Lỗi trên hot path: ghi diagnostics, trả `consumed = false`; không bao giờ làm mất phím | Review + golden test |
| Hot path (`Policy::predict`, `FeatureExtractor::extract` sau khi ấm) không cấp phát | Review, benchmark |
| Không đọc wall clock trong engine và crate thuần | `engine-event-clock`, `pure-no-side-effects` |
| File chỉ qua `Storage` | `engine-not-up`, `leaf-policy`, `ffi-thin`, `pure-no-side-effects` |
| Đổi API công khai, schema sự kiện, config mặc định → cập nhật `docs/` cùng commit | Review ([README](README.md)) |
| Đổi schema không tương thích → tăng `SCHEMA_VERSION` | Review |

## Test

| Crate | Loại test |
| --- | --- |
| `nimk-types` | Unit: `CaretRel::new`/`cell`, `Lang`, round-trip serde của `Record` |
| `nimk-telex` | Bảng `raw` → `vi`; bảng âm tiết hợp lệ/không hợp lệ; bất biến `transform(&encode(w)).text == w` |
| `nimk-storage` | Round-trip `MemStorage` và `PlainFsStorage` (`tempfile`) |
| `nimk-policy` | Seed cố định; học được tập đồ chơi (`has` trong app A là EN, app B là VI); `save`/`load` round-trip, header sai → `Corrupt` |
| `nimk-engine` | Golden test với `Threading::Inline`: chuỗi phím (kèm hotkey) → chuỗi `EngineOutput` và chuỗi record |
| `nimk-synth` | Cùng `SynthConfig` + seed → cùng script; `word_id` khớp id engine gán |
| `nimk-replay` | Fixture log nhỏ → metric kỳ vọng |
| `nimk-ffi` | Smoke test gọi qua C ABI từ Rust test |
