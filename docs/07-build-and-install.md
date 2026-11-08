# Build & cài đặt

> **Trạng thái:** draft · **Cập nhật:** 2026-11-08 · **Code:** `Makefile`, `Cargo.toml`, `scripts/`, `crates/nimk-ffi`, `crates/nimk-synth/src/bin/nimk-synth.rs`

Build không dùng Xcode project: `cargo` tạo `libnimk_ffi.a`, `cbindgen` sinh header, `swiftc` link thành `NeuralIMK.app`.
`make check` phải xanh trước mỗi commit.

## Target của `Makefile`

| Target | Lệnh | Ghi chú |
| --- | --- | --- |
| `check` | `fmt lint test arch` | Cổng trước commit |
| `fmt` | `cargo fmt --all -- --check` | |
| `lint` | `cargo clippy --workspace --all-targets -- -D warnings` | Workspace lint cấm `unwrap_used`, `expect_used`, `print_stdout` |
| `test` | `cargo test --workspace` | |
| `arch` | `cargo arch-lint check` rồi `cargo arch-lint check --rules no-panic-in-lib` | Contract trong `arch-lint.toml` (arch-lint 0.11), xem [08](08-coding-standard.md) |
| `header` | `cbindgen --config crates/nimk-ffi/cbindgen.toml --crate nimk-ffi --output target/nimk_ffi.h` | Sinh header C cho Swift |
| `app` | `./scripts/build-app.sh` | Đóng gói `NeuralIMK.app` |
| `install` | `app` rồi `./scripts/install.sh` | Copy vào `~/Library/Input Methods/`; dừng bản cũ bằng SIGTERM nên engine flush trước khi thoát |

`scripts/build-app.sh`, `scripts/install.sh`, `scripts/make-icon.swift` (icon template "Vi") và `crates/nimk-ffi/cbindgen.toml` đã có; `/build` nằm trong `.gitignore`.
IME mới cài và chưa kiểm chứng trên máy thật; xem [Chưa kiểm chứng](02-keyboard-io.md#lifecycle-imk) và [09](09-app-compat.md).

## Pipeline build

```text
+-------------------+  cargo build   +------------------+
| crates/nimk-ffi   | -------------> | libnimk_ffi.a    | ---+
+-------------------+  --release     +------------------+    |
        |                                                    |  link
        | cbindgen                                           v
        v                            +------------------+  +---------------------------+
+-------------------+  import        | macos/NeuralIMK/ |  | NeuralIMK.app/Contents/   |
| target/nimk_ffi.h | -------------> | *.swift (swiftc) |->|   MacOS/NeuralIMK         |
+-------------------+  objc-header   +------------------+  |   Info.plist              |
                                                           +---------------------------+
                                                                  | codesign -s -
                                                                  v
                                                     ~/Library/Input Methods/
```

Header là hợp đồng duy nhất giữa hai ngôn ngữ; đổi `nimk-ffi` mà quên `make header` thì Swift build lỗi ngay.

`scripts/build-app.sh` làm theo thứ tự: sinh header, `cargo build --release -p nimk-ffi`, `swiftc -O` link thư viện và các framework `Cocoa`, `InputMethodKit`, `CoreGraphics`, ghép `Info.plist`, vẽ icon, rồi `codesign -s -` (ad hoc).
Biến môi trường `ARCHS="arm64 x86_64"` cho bản universal (dùng `lipo`); mặc định chỉ kiến trúc của máy; macOS tối thiểu là 13.0.

## Profile build

Dependency chạy tối ưu hoá trong cả build dev vì `burn` chậm khoảng 40 lần khi không tối ưu.
`Cargo.toml` gốc đặt `[profile.dev.package."*"] opt-level = 3`; code của workspace vẫn là debug.

```toml
# burn is ~40x slower unoptimized; keep dependencies fast in dev/test builds.
[profile.dev.package."*"]
opt-level = 3
```

`nimk-policy` phụ thuộc `burn = { version = "=0.22.1", default-features = false, features = ["std", "flex", "autodiff", "optim"] }` ([ADR 0009](adr/0009-burn-nn-backend.md)).
Tắt feature mặc định nghĩa là không rayon: kernel chạy đơn luồng, hợp với process IME. Không dùng backend GPU.
Test học `GruAction` chỉ chạy ở bản release (273 s ở debug): `cargo test --release -p nimk-policy` hoặc `--include-ignored`; bài đo latency cần `--release ... --ignored`.

## Pretrain model

IME cần một snapshot đã pretrain trước lần cài đầu; model chưa train là trung tính và không commit từ đúng ([04](04-engine-ml.md)).
Quy trình dưới đây chưa được chạy đầu-cuối trên máy cài thật.

1. Pretrain vào một thư mục tạm, không bao giờ vào thư mục dữ liệu thật khi chưa chủ ý (`--out` là bắt buộc):

   ```sh
   cargo run --release -p nimk-synth --bin nimk-synth -- pretrain \
       --model gru_action --out /tmp/nimk-pre --sessions 200 --words 500 --epochs 2
   ```

2. Đọc báo cáo in ra (`edit_accuracy`, `render_accuracy`, `ambiguous_acc`, `text_match_rate`); kỳ vọng xem [04](04-engine-ml.md#kết-quả-đo).
3. Snapshot nằm ở `/tmp/nimk-pre/blobs/model/gru_action/latest`. Copy nó vào `~/Library/Application Support/NeuralIMK/blobs/model/gru_action/latest` trước khi cài IME lần đầu (tạo thư mục nếu chưa có).
4. Cài IME. `Engine::new` nạp snapshot này; nếu hỏng thì dựng model mới và báo qua diagnostics.
5. So sánh offline: `nimk-replay --data <dir> --init /tmp/nimk-pre --model gru_action [--frozen]`.

`nimk-synth gen --out DIR` ghi event log synthetic thay vì model; flag chung `--sessions N --words N --seed S`, riêng `pretrain` có `--model`, `--epochs N`, `--eval-frac F`, `--batch N`.
Mặc định 100 session x 500 từ, seed 42, 2 epoch, `eval-frac` 0.1, batch 128; bản GRU 200 x 500 x 2 mất khoảng 102 s.
Snapshot cũ từ trước khi sửa rò nhãn `SHOWN_VI` phải pretrain lại ([04](04-engine-ml.md#encoding)).

## `Info.plist`

Thiếu hoặc sai một key dưới đây thì macOS không liệt kê IME trong Input Sources.

| Key | Giá trị | Ghi chú |
| --- | --- | --- |
| `CFBundleIdentifier` | `com.giangndm.inputmethod.NeuralIMK` | Phải chứa `.inputmethod.` |
| `InputMethodConnectionName` | `com.giangndm.inputmethod.NeuralIMK_Connection` | Truyền cho `IMKServer` trong `main.swift` |
| `InputMethodServerControllerClass` | `InputController` | Swift khai báo `@objc(InputController)` để tránh namespace module |
| `LSBackgroundOnly` | `true` | Không icon Dock |
| `TISIntendedLanguage` | `vi` | |
| `tsInputMethodCharacterRepertoireKey` | `["Latn"]` | |

## Cài đặt

0. Lần đầu: copy snapshot đã pretrain vào thư mục dữ liệu ([Pretrain model](#pretrain-model)).
1. `make install` (build, đóng gói, copy vào `~/Library/Input Methods/`).
2. `killall NeuralIMK` để process cũ thoát; hệ thống tự chạy bản mới khi IME được chọn.
3. Lần đầu: System Settings → Keyboard → Input Sources → thêm NeuralIMK.
4. Giữ input source ABC bật làm đường lùi.

## Debug & phục hồi

| Tình huống | Làm gì |
| --- | --- |
| Xem log debug | `~/Library/Logs/NeuralIMK.log` (menu IME → `Show diagnostics log`) |
| Xem dữ liệu, event log | `~/Library/Application Support/NeuralIMK/` (hoặc menu IME → `Open data folder`) |
| IME treo hoặc crash, không gõ được | Chuyển sang ABC bằng menu input; `killall NeuralIMK` |
| IME không hiện trong Input Sources | Kiểm tra `Info.plist` ở bảng trên; log out/in để hệ thống quét lại |
| Bản mới không có hiệu lực | `killall NeuralIMK`, chọn lại input source |
| Model hỏng, `load` lỗi | Engine tự dựng model mới (chưa pretrain nên gõ sai); xoá `blobs/model/<kind>/latest` rồi copy snapshot pretrain lại |
| Gỡ cài đặt | Xoá `~/Library/Input Methods/NeuralIMK.app`, bỏ khỏi Input Sources |

Lỗi trong hot path không bao giờ làm mất phím: Engine trả `consumed = false` và phím đi thẳng vào app.
