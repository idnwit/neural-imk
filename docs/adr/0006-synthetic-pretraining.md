# 0006. Pretrain bằng dữ liệu gõ synthetic

> **Trạng thái:** accepted · **Ngày:** 2026-11-08

## Bối cảnh

Học online từ con số 0 nghĩa là những ngày đầu model đoán gần ngẫu nhiên ở vùng mập mờ, và người dùng phải sửa nhiều.
Log thật chỉ có sau M3, mà M4 cần dữ liệu để so sánh policy và benchmark.
Phần lớn tín hiệu "từ này là VI hay EN" nằm ở chính chuỗi phím, không cần context app.

## Quyết định

- Thêm crate `nimk-synth` sinh script gõ VI/EN trộn kiểu người Việt (văn xuôi VI chen thuật ngữ EN, câu EN chen từ VI), không có context app (`SYNTHETIC_BUNDLE_ID = "synthetic"`).
- Chuỗi phím Telex sinh bằng `nimk_telex::encode`, nghịch đảo của `transform`; từ EN mập mờ (`has`, `for`, `user`) được lấy mẫu dày hơn.
- Hai đầu ra: `write_log` ghi thành event log thường để `nimk-replay` đọc; `pretrain` cho script chạy qua engine inline (`hotkeys = false`, nhãn qua `Engine::inject_label`) rồi lưu model làm snapshot khởi đầu của IME.
- Corpus viết tay cho dự án, không dùng corpus bên thứ ba; cùng `SynthConfig` + seed cho cùng output.

## Hệ quả

- IME khởi động với model đã biết phân biệt n-gram VI/EN; online learning chỉ còn học phần riêng theo app và người dùng.
- Có benchmark tất định cho policy trước khi có log thật.
- Feature app/vị trí không học được từ synthetic; model phải học chúng online.
- Phân phối synthetic lệch với cách người dùng thật gõ; metric trên synthetic không thay được metric dùng thật.
- Nhãn synthetic ghi là `correction` weight 1.0, nên log synthetic phải tách khỏi log thật khi tính metric.
