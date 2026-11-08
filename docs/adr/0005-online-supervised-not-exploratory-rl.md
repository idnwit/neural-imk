# 0005. Học có giám sát online, không dùng RL có exploration

> **Trạng thái:** accepted · **Ngày:** 2026-11-08

## Bối cảnh

Tên repo gợi ý RL, nhưng tín hiệu thực có là nhãn ngầm: hotkey nghĩa là "sai, đúng ra là mode kia", im lặng nghĩa là "có lẽ đúng".
Exploration (thử mode khác để học) nghĩa là cố ý gõ sai trước mắt người dùng, trực tiếp tăng metric chính.
Hành động chỉ có hai và nhãn đúng biết được ngay sau cửa sổ sửa, nên không có bài toán gán công trễ.

## Quyết định

- Mô hình hoá thành phân loại nhị phân `p(VI | context)`, học online từ `Sample { features, label, weight }`.
- ε = 0; chống dao động bằng hysteresis `switch_margin` (0.2).
- Nhãn chốt sau cửa sổ sửa (2 từ hoặc 10 s): không sửa → weight 0.3; sửa bằng hotkey → weight 1.0 (0.5 cho từ thứ hai).
- Correction train lại 20 sample gần nhất trong 5 bước; replay buffer 10 000 chống quên.
- Tách `Policy` (đọc, hot path) và `Learner` (ghi, trainer thread).

## Hệ quả

- Không bao giờ cố ý gõ sai để học; người dùng chỉ thấy lỗi khi model thật sự nhầm.
- Nhãn ngầm thiên lệch theo policy cũ: replay là ước lượng, kết luận cuối dựa trên dùng thật ([05](../05-storage-and-log.md)).
- Vùng mà policy chưa bao giờ chọn mode kia chỉ học được khi người dùng tự sửa.
- Có thể nâng lên contextual bandit sau nếu dữ liệu cho thấy cần, chỉ bằng ADR mới.
