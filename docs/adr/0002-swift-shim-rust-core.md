# 0002. Swift shim mỏng, Rust core link tĩnh qua C ABI

> **Trạng thái:** accepted · **Ngày:** 2026-11-08

## Bối cảnh

InputMethodKit và AppKit chỉ dùng thuận tiện từ Swift/Objective-C.
Logic chính (composer, Telex, policy, trainer, log) cần test thuần, replay tất định và kiểm soát cấp phát trên hot path.
Bỏ marked text (commit từng ký tự rồi backspace giả để sửa) dễ vỡ trên nhiều app.

## Quyết định

- Một process `NeuralIMK.app`: Swift shim (≤ 300 dòng) + `libnimk_ffi.a` link tĩnh.
- Swift chỉ chuyển `NSEvent`/context sang Rust và thực thi `NimkOut`; không có logic quyết định trong Swift.
- `nimk-ffi` là crate duy nhất được dùng `unsafe` và `extern "C"`; mọi hàm export bọc `catch_unwind` và lỗi thì trả `consumed = false`.
- Header sinh bằng `cbindgen`; build bằng `cargo` + `swiftc`, không dùng Xcode project.
- Giữ từ đang gõ dưới dạng marked text; chấp nhận gạch chân của preedit.

## Hệ quả

- Toàn bộ hành vi test được bằng `cargo test` và replay mà không cần AppKit.
- Đổi mode giữa từ chỉ là thay preedit; không cần backspace giả.
- Hai ngôn ngữ, một ranh giới ABI: chuỗi do Rust sở hữu sống đến lần gọi kế, Swift phải copy ngay.
- Gạch chân luôn hiện dưới từ đang gõ; ẩn nó nằm ngoài phạm vi PoC.
