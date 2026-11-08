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

App đóng gói sẵn model pretrain trên dữ liệu synthetic; lần chạy đầu bắt đầu từ chúng và học tiếp online ([ADR 0010](adr/0010-sdk-storage-and-bundled-models.md)).

1. `make models` pretrain `linear_action` và `gru_action` (seed 42, 200 session x 500 từ, 2 epoch) vào `target/models/<kind>/`, rồi copy mỗi checkpoint tới `target/models/bundle/model/<kind>/latest`.
2. Kind nào đã có bản trong `bundle/` thì bỏ qua; `make models FORCE=1` pretrain lại (GRU mất khoảng 100 s).
3. `scripts/build-app.sh` chạy `make models` nếu `bundle/` thiếu, rồi copy nó vào `NeuralIMK.app/Contents/Resources/models/` (khoảng 0.8 MB). File sinh ra nằm dưới `target/`, không commit.
4. Lần chạy đầu: `ReadOnlyDirBlobs` phục vụ model gốc, menu ghi `(gốc)`. Khi học, checkpoint ghi vào `<Application Support>/<bundle id>/blobs/model/<kind>/latest`, menu ghi `(đã học thêm)`.
5. Menu bộ gõ > `Khôi phục model gốc` (các mục model nằm thẳng ở menu chính, dạng `Model: GRU (gốc) ✓`) xoá checkpoint người dùng của model đang dùng và nạp lại bản gốc.

Pretrain thủ công vẫn dùng được; `--out` là bắt buộc và không bao giờ mặc định vào data dir:

```sh
cargo run --release -p nimk-synth --bin nimk-synth -- pretrain \
    --model gru_action --out target/models/gru_action --sessions 200 --words 500 --epochs 2
```

Đọc báo cáo in ra (`edit_accuracy`, `render_accuracy`, `ambiguous_acc`, `text_match_rate`); kỳ vọng xem [04](04-engine-ml.md#kết-quả-đo).
So sánh offline: `nimk-replay --data <dir> --init target/models/gru_action --model gru_action [--frozen]`.

`nimk-synth gen --out DIR` ghi event log synthetic thay vì model; flag chung `--sessions N --words N --seed S`, riêng `pretrain` có `--model`, `--epochs N`, `--eval-frac F`, `--batch N`.
Mặc định 100 session x 500 từ, seed 42, 2 epoch, `eval-frac` 0.1, batch 128.
Snapshot cũ từ trước khi sửa rò nhãn `SHOWN_VI` phải pretrain lại ([04](04-engine-ml.md#encoding)).

## Vị trí lưu trữ

Mọi vị trí do SDK macOS trả về, không viết cứng trong Swift: data dir là `FileManager.url(for: .applicationSupportDirectory)` nối bundle id; log là `.libraryDirectory`/`Logs`/`<bundle id>.log`; model gốc là `Bundle.main.resourceURL/models`.
Ngoại lệ duy nhất là `scripts/install.sh` copy vào `$HOME/Library/Input Methods`, vị trí Apple quy định cho input method của người dùng.

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

1. `make install` (pretrain model nếu chưa có, build, đóng gói kèm model, copy vào `$HOME/Library/Input Methods/`).
2. `killall NeuralIMK` để process cũ thoát; hệ thống tự chạy bản mới khi IME được chọn.
3. Lần đầu: System Settings → Keyboard → Input Sources → thêm NeuralIMK.
4. Giữ input source ABC bật làm đường lùi.

## Debug & phục hồi

| Tình huống | Làm gì |
| --- | --- |
| Xem log debug | Menu IME → `Show diagnostics log` (`<Library>/Logs/<bundle id>.log`), hoặc `log stream --predicate 'subsystem == "com.giangndm.inputmethod.NeuralIMK"'` |
| Xem dữ liệu, event log | Menu IME → `Open data folder` (`<Application Support>/<bundle id>`) |
| IME treo hoặc crash, không gõ được | Chuyển sang ABC bằng menu input; `killall NeuralIMK` |
| IME không hiện trong Input Sources | Kiểm tra `Info.plist` ở bảng trên; log out/in để hệ thống quét lại |
| Bản mới không có hiệu lực | `killall NeuralIMK`, chọn lại input source |
| Model học sai hoặc checkpoint hỏng | Menu bộ gõ > `Khôi phục model gốc`; checkpoint hỏng thì engine dựng model mới và báo diagnostics |
| Gỡ cài đặt | Xoá `$HOME/Library/Input Methods/NeuralIMK.app`, bỏ khỏi Input Sources |

Lỗi trong hot path không bao giờ làm mất phím: Engine trả `consumed = false` và phím đi thẳng vào app.
