# 0009. Backend neural network: burn 0.22.1 (flex)

> **Trạng thái:** accepted · **Ngày:** 2026-11-08

## Bối cảnh

[ADR 0007](0007-neural-action-model.md) cần một model neural chạy mỗi phím với p99 < 1 ms và học online trong process IME.
Backend phải suy luận và huấn luyện bằng Rust thuần, không Python, không runtime ngoài, không GPU.

Đã đo thử GRU nhỏ (embedding 32, hidden 64) trên M4 Pro:

| Ứng viên | Suy luận GRU (số đo ghi trong khảo sát) | Bước Adam, batch 32 | Nhận xét |
| --- | --- | --- | --- |
| `burn` 0.22.1 backend `flex` | p99 108 µs | khoảng 2.5 ms | Có sẵn `Module`, autodiff, optimizer, record |
| `candle` | 96 / 127 µs | khoảng 2.0 ms | Cùng bậc với burn |
| `tract` | — | — | Chỉ suy luận, không huấn luyện |
| `ort` (ONNX Runtime) | — | — | Huấn luyện cần artifact sinh bằng Python |
| `tch` (libtorch) | — | — | Libtorch quá nặng để đóng gói vào IME |
| `dfdx` | — | — | Không còn được duy trì |

Hai ứng viên khả thi là `burn` và `candle`, chênh lệch nhỏ so với ngân sách 1 ms.

## Quyết định

- Dùng `burn = "=0.22.1"` với `default-features = false` và features `std`, `flex`, `autodiff`, `optim`, cho `GruAction` trong `nimk-policy`.
- Tắt feature mặc định nên không có `rayon`: kernel đơn luồng, không tranh CPU với app đang gõ; không backend GPU. Feature `simd` không cải thiện đo được, `store` không cần.
- Ghim phiên bản chính xác (`=`) vì API của burn đổi giữa các bản nhỏ.
- `LinearAction` viết tay giữ làm đường cơ sở và model dự phòng; hai model cùng nằm sau `ImeModel`/`ImeLearner`, nên đổi backend không đụng engine.
- RNG: `GruAction` giữ RNG riêng theo `seed` (khởi tạo và context dropout) vì RNG thiết bị của burn là global của process.
- Bản debug chậm khoảng 40 lần, nên `Cargo.toml` gốc đặt `[profile.dev.package."*"] opt-level = 3` ([07](../07-build-and-install.md)).

## Hệ quả

- Độ trễ đo trong sản phẩm: `score` trung bình 166 µs, p99 khoảng 170 đến 186 µs; `update` batch 32 là 3.6 ms (p99 4.3 ms) trên trainer thread; snapshot 4 µs.
- Snapshot `GruAction` lưu bằng record của burn nên byte không tất định (id tham số ngẫu nhiên): so sánh hai snapshot phải theo điểm số, không theo byte. Trạng thái Adam không được lưu.
- Rủi ro API churn: nâng cấp burn là việc có chủ đích, kèm chạy lại test học và benchmark.
- Rủi ro `flex` còn mới: nếu gặp lỗi số học hoặc hiệu năng, quay về `candle` (cùng bậc độ trễ) hoặc dùng `LinearAction` mà không đổi engine.
- Cần giới hạn số luồng nếu sau này bật `rayon`; việc đặt QoS thấp cho trainer thread (utility) chưa làm và là việc mở.
- Thời gian build và kích thước `libnimk_ffi.a` tăng do burn; test học `GruAction` chỉ chạy ở bản release.
