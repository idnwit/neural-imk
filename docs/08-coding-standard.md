# Coding standard

> **Trạng thái:** stable · **Cập nhật:** 2026-11-08 · **Code:** `Cargo.toml`, `clippy.toml`, `arch-lint.toml`, `Makefile`

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
| `clippy::print_stdout` | deny | `[workspace.lints.clippy]` | Không `println!`; `nimk-replay` (`main.rs`) và `nimk-synth` (`bin/nimk-synth.rs`) được `allow` vì in report |
| `allow-unwrap-in-tests`, `allow-expect-in-tests` | true | `clippy.toml` | Test được `unwrap`/`expect` |

Mỗi crate trừ `nimk-ffi` còn có `#![forbid(unsafe_code)]` ở `lib.rs`/`main.rs`; `nimk-ffi` đặt `#![allow(unsafe_code)]` và mỗi khối `unsafe` có comment `SAFETY`.
Code dùng `match`/`if let` thay cho `unwrap_or` vì rule AL013 của arch-lint 0.11 báo mọi `unwrap_or`; ngoại lệ có chủ đích ghi bằng `// arch-lint: allow(<rule>) reason="..."`.

## Contract arch-lint

`arch-lint.toml` (arch-lint 0.11, `preset = "recommended"`, `fail_on = "warning"`) khoá tầng và hot path theo đường dẫn `use`.

### Scope

| Scope | Đường dẫn |
| --- | --- |
| `types`, `storage`, `telex`, `policy`, `engine`, `ffi`, `replay`, `synth` | `crates/nimk-<tên>/src/**` |
| `pure` | `nimk-types`, `nimk-telex`, `nimk-policy` |
| `eng-composer`, `eng-hotkey`, `eng-labeler`, `eng-trainer`, `eng-logger` | Từng file `crates/nimk-engine/src/<module>.rs` (`eng-trainer` gồm `trainer.rs` và `trainer/**`) |
| `engine-no-threads` | `crates/nimk-engine/src/**` (gồm `engine/flow.rs`, `engine/switch.rs`) trừ `trainer.rs`, `trainer/**`, `logger.rs` |
| `non-ffi` | `crates/*/src/**` trừ `crates/nimk-ffi/src/**` |

### Quy tắc

| Quy tắc | Loại | Scope | Cấm | Bảo vệ điều gì |
| --- | --- | --- | --- | --- |
| `types-is-bottom` | `restrict-use` | `types` | `nimk_storage`, `nimk_telex`, `nimk_policy`, `nimk_engine`, `nimk_ffi`, `nimk_synth`, `nimk_replay` | `nimk-types` là tầng đáy |
| `leaf-storage` | `restrict-use` | `storage` | `nimk_types`, `nimk_telex`, `nimk_policy`, `nimk_engine`, `nimk_ffi`, `nimk_synth`, `nimk_replay` | Storage chỉ biết bytes, giữ `EncryptedStorage` là decorator thuần |
| `leaf-telex` | `restrict-use` | `telex` | `nimk_storage`, `nimk_policy`, `nimk_engine`, `nimk_ffi`, `nimk_synth`, `nimk_replay` | Telex chỉ phụ thuộc `nimk-types` |
| `leaf-policy` | `restrict-use` | `policy` | `nimk_storage`, `nimk_telex`, `nimk_engine`, `nimk_ffi`, `nimk_synth`, `nimk_replay`, `std::fs`, `std::thread` | Policy chỉ map `Observation` → điểm action |
| `engine-not-up` | `restrict-use` | `engine` | `nimk_ffi`, `nimk_synth`, `nimk_replay`, `std::fs` | Engine không biết tầng trên, ghi file qua `Storage` |
| `ffi-thin` | `restrict-use` | `ffi` | `nimk_policy`, `nimk_telex`, `nimk_synth`, `nimk_replay`, `std::fs` | FFI chỉ adapt `Engine` (và dựng `PlainFsStorage`) sang C |
| `replay-not-ffi` | `restrict-use` | `replay` | `nimk_ffi` | Replay gọi `Engine` trực tiếp |
| `synth-not-ffi` | `restrict-use` | `synth` | `nimk_ffi` | Synth gọi `Engine` trực tiếp |
| `pure-no-side-effects` | `restrict-use` | `pure` | `std::fs`, `std::io`, `std::net`, `std::thread`, `std::sync` (trừ `Arc`, `atomic`), `std::time`, `std::env`, `parking_lot` | Crate thuần: không I/O, thread, lock, env, wall clock |
| `engine-event-clock` | `restrict-use` | `engine` | `std::time`, `std::env` | Đồng hồ duy nhất là timestamp của event (replay tất định) |
| `hot-path-no-io` | `restrict-use` | `engine-no-threads` | `std::fs`, `std::net`, `std::thread`, `Mutex`, `RwLock`, `Condvar` | Đường phím không I/O, lock, thread |
| `eng-composer-independent` | `restrict-use` | `eng-composer` | `hotkey`, `labeler`, `trainer`, `logger`, `engine` (dạng `crate::<mod>` và `super::<mod>`) | Module engine độc lập |
| `eng-hotkey-independent` | `restrict-use` | `eng-hotkey` | `composer`, `labeler`, `trainer`, `logger`, `engine` | Như trên |
| `eng-labeler-independent` | `restrict-use` | `eng-labeler` | `composer`, `hotkey`, `trainer`, `logger`, `engine` | Như trên |
| `eng-trainer-independent` | `restrict-use` | `eng-trainer` | `composer`, `hotkey`, `labeler`, `logger`, `engine` | Như trên |
| `eng-logger-independent` | `restrict-use` | `eng-logger` | `composer`, `hotkey`, `labeler`, `trainer`, `engine` | Như trên |
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
| `no-panic-in-lib` (AL011) không nằm trong preset `recommended`, `enabled = true` không thêm được | `make arch` chạy pass thứ hai với `--rules no-panic-in-lib` |
| AL013 báo mọi `unwrap_or` | Dùng `match`/`if let`; cân nhắc chỉnh rule này |
| Scope theo file không bao phủ file con | Thêm glob thư mục con vào scope (ví dụ `trainer/**`) mỗi khi tách module thành thư mục |

### Thêm quy tắc

1. Thêm scope/quy tắc vào `arch-lint.toml`, kèm `message` nói lý do.
2. Viết probe vi phạm, chạy `make arch`, xác nhận bị chặn, rồi xoá probe.
3. Cập nhật bảng trên và [01-architecture.md](01-architecture.md) trong cùng commit.

## Quy ước code

| Quy ước | Kiểm bởi |
| --- | --- |
| Swift shim không có logic quyết định; đổi hành vi làm trong Rust, kèm test | Review |
| Lỗi trên hot path: ghi diagnostics, trả `consumed = false`; không bao giờ làm mất phím | Review + golden test |
| Hot path (`ImeModel::score`, `Encoded::new`) không cấp phát lớn và p99 < 1 ms | Review, `tests/action_latency.rs` (ignored, chạy tay) |
| Không đọc wall clock trong engine và crate thuần | `engine-event-clock`, `pure-no-side-effects` |
| File chỉ qua `Storage` | `engine-not-up`, `leaf-policy`, `ffi-thin`, `pure-no-side-effects` |
| Đổi API công khai, schema sự kiện, config mặc định → cập nhật `docs/` cùng commit | Review ([README](README.md)) |
| Đổi schema không tương thích → tăng `SCHEMA_VERSION` | Review |

## Test

`make check` hiện chạy 187 test xanh (185 test thường + 2 doc-test của `nimk-telex`) và 2 test `#[ignore]` của `nimk-policy`.

| Crate | Số test | Loại test |
| --- | --- | --- |
| `nimk-types` | 4 | Unit: `CaretRel::new`/`cell`, `Lang` |
| `nimk-storage` | 19 | 7 unit; 12 integration chạy cùng một bộ test trên `MemStorage` và `PlainFsStorage` (`tempfile`) |
| `nimk-telex` | 13 + 2 doc | 10 unit; 3 integration: `transform(&encode(w)).text == w` trên hơn 5000 âm tiết tổ hợp (thường, Title, HOA), mọi tiền tố viable |
| `nimk-policy` | 18 (+2 ignored) | `action_encode` 6, `action_model` 9 (neutral init, save/load, `Corrupt`, `decide`), `action_learn` 1 chạy ở release (bản debug bị `ignore`), 2 unit codec; `action_latency` ignored, chạy tay |
| `nimk-engine` | 69 | 18 unit (hotkey, composer, labeler, trainer, logger); integration `golden` 19 (chuỗi phím → `EngineOutput` và record, `Threading::Inline`), `retro` 15 (sửa ngược), `robust` 7 (20 000 sự kiện ngẫu nhiên không panic), `teach` 4, `commit` 3, `fresh` 3 |
| `nimk-ffi` | 18 | Unit qua C ABI: null-safe, `catch_unwind`, `NimkOut` |
| `nimk-replay` | 14 | 7 unit (CLI, load, metric, report), 7 integration (fixture log nhỏ → metric kỳ vọng) |
| `nimk-synth` | 30 | 28 unit (generate, `targets`, `write_log`, pretrain, eval) + 2 test CLI |

Quy ước test:

- Seed cố định; `Threading::Inline` cho mọi test cần tất định.
- Test học nặng (`GruAction`) chỉ chạy ở bản release; `Cargo.toml` gốc bật `opt-level = 3` cho dependency để test debug còn dùng được ([07](07-build-and-install.md)).
- Đổi hành vi engine phải có golden hoặc test tích hợp tương ứng; hành vi sửa ngược nằm ở `tests/retro.rs`.
