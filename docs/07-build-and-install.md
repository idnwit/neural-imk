# Build & cài đặt

> **Trạng thái:** draft · **Cập nhật:** 2026-11-08 · **Code:** `Makefile`, `scripts/`, `crates/nimk-ffi`

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
| `install` | `app` rồi `./scripts/install.sh` | Copy vào `~/Library/Input Methods/` |

`scripts/`, `macos/` và `crates/nimk-ffi/cbindgen.toml` chưa có ở M0; `header`, `app`, `install` chạy được từ M2.

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

Các bước của `scripts/build-app.sh` (minh hoạ):

```sh
# minh hoạ
cargo build --release -p nimk-ffi
make header
swiftc -O \
  -import-objc-header macos/NeuralIMK/NeuralIMK-Bridging.h \
  macos/NeuralIMK/*.swift \
  target/release/libnimk_ffi.a \
  -framework InputMethodKit -framework Cocoa \
  -o build/NeuralIMK.app/Contents/MacOS/NeuralIMK
cp macos/NeuralIMK/Info.plist build/NeuralIMK.app/Contents/
codesign -s - --force build/NeuralIMK.app
```

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

1. `make install` (build, đóng gói, copy vào `~/Library/Input Methods/`).
2. `killall NeuralIMK` để process cũ thoát; hệ thống tự chạy bản mới khi IME được chọn.
3. Lần đầu: System Settings → Keyboard → Input Sources → thêm NeuralIMK.
4. Giữ input source ABC bật làm đường lùi.

## Debug & phục hồi

| Tình huống | Làm gì |
| --- | --- |
| Xem log debug | `~/Library/Logs/NeuralIMK.log` |
| Xem dữ liệu, event log | `~/Library/Application Support/NeuralIMK/` (hoặc menu IME → "Mở thư mục dữ liệu") |
| IME treo hoặc crash, không gõ được | Chuyển sang ABC bằng menu input; `killall NeuralIMK` |
| IME không hiện trong Input Sources | Kiểm tra `Info.plist` ở bảng trên; log out/in để hệ thống quét lại |
| Bản mới không có hiệu lực | `killall NeuralIMK`, chọn lại input source |
| Model hỏng, `load` lỗi | Menu IME → "Reset model", hoặc xoá snapshot trong `blobs/` |
| Gỡ cài đặt | Xoá `~/Library/Input Methods/NeuralIMK.app`, bỏ khỏi Input Sources |

Lỗi trong hot path không bao giờ làm mất phím: Engine trả `consumed = false` và phím đi thẳng vào app.
