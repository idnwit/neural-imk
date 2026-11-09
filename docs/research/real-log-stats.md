# Số đo log gõ thật

> **Trạng thái:** draft · **Cập nhật:** 2026-11-09 · **Code:** `crates/nimk-synth/examples/log_collisions.rs`

Chỉ số tổng hợp từ event log của một người dùng (2026-11-08 đến 2026-11-09, 4 file, 23 112 dòng, **1319 từ commit**). Mẫu rất nhỏ và còn sớm: gần như toàn bộ từ nằm trong terminal, các nhóm khác chỉ vài chục từ. Không có từ hay văn bản gõ nào trong repo này; mọi con số dưới đây đều là đếm hoặc tỉ lệ.

## Phương pháp

Mỗi `word_commit` được gán ngôn ngữ cuối cùng: nhãn `label` của từ nếu có (1297 implicit, 19 correction), không thì `mode` lúc commit. Nhóm app suy từ `bundle_id` của `context` gần nhất (terminal, IDE, browser, khác). Run là chuỗi từ liên tiếp cùng ngôn ngữ trong một session. "Va chạm" (collision) là từ EN mà raw keys qua Telex ra âm tiết VI hợp lệ khác raw (`nimk_telex::transform`: `has_telex && valid_vi`). Số liệu run và nhóm app tính bằng script ngoài repo; va chạm tính bằng `examples/log_collisions.rs` (in số đếm, không in từ).

## Kết quả

| Nhóm | Từ | % VI | Run VI (trung bình) | Run EN (trung bình) | Liền kề cùng ngôn ngữ | raw = rendering VI | Chuyển tay / 1000 từ |
| --- | --- | --- | --- | --- | --- | --- | --- |
| terminal | 1261 | 71.1 | 4.5 | 1.8 | 0.686 | 30.7% | 15.1 (19 lần) |
| IDE | 11 | 45.5 | n quá nhỏ | n quá nhỏ | 0.80 | 27.3% | 0 |
| browser | 17 | 11.8 | n quá nhỏ | n quá nhỏ | 0.75 | 70.6% | 176 (3 lần) |
| khác | 29 | 75.9 | n quá nhỏ | n quá nhỏ | 0.71 | 44.8% | 0 |
| tất cả | 1319 | 70.2 | 4.43 | 1.90 | 0.688 | 31.5% | 16.7 (22 lần) |

Phân bố độ dài run (tất cả; số run độ dài 1, 2, 3, 4+):

| Ngôn ngữ | 1 | 2 | 3 | 4+ |
| --- | --- | --- | --- | --- |
| VI | 46 | 40 | 20 | 103 |
| EN | 133 | 45 | 10 | 19 |

Va chạm: 394 từ EN, 79 (20.1%) có rendering VI hợp lệ khác raw, tức 6.0% mọi từ. 47 trên 925 từ VI (5.1%) có raw trùng một từ trong corpus EN của generator. Telex đổi 733 trên 925 từ VI (79%), phần còn lại là từ VI không có dấu hay chữ đặc biệt.

Tỉ lệ sửa: 22 lần chuyển tay trên 1319 từ (16.7 mỗi 1000 từ); 19 nhãn `correction`.

Từ hay gặp: nhóm EN gồm tên lệnh, từ kỹ thuật và vài từ chức năng ngắn, nhóm VI chủ yếu là từ chức năng (liên từ, trợ từ, đại từ). Từ chức năng ngắn chiếm phần lớn số lần xuất hiện của cả hai ngôn ngữ, đúng giả thuyết trong [right-context-experiment](right-context-experiment.md).

## Dùng cho generator

| Thống kê | Thật | `MixProfile::legacy` | `MixProfile::default` |
| --- | --- | --- | --- |
| % VI | 70.2 | 60.4 | 68.9 |
| Run VI / EN | 4.4 / 1.9 | 5.0 / 3.3 | 4.5 / 2.0 |
| Liền kề cùng ngôn ngữ | 0.688 | 0.759 | 0.695 |
| Va chạm / từ EN | 20.1% | 40.1% | 19.2% |
| Va chạm / mọi từ | 6.0% | 15.9% | 6.0% |
| raw = rendering | 31.5% | 22.3% | 27.2% |

Generator cũ cho va chạm gấp đôi thật vì `p_ambiguous` 0.4 ép lấy từ EN mập mờ; vì vậy eval "ambiguous" trên synthetic cũ khó hơn log thật về mật độ, nhưng dễ hơn về cụm. Profile mới khớp thật trên các chỉ số này (xem [04](../04-engine-ml.md#dữ-liệu-synthetic-thực-tế)).

## Giới hạn

- 1319 từ từ một người, một tuần đầu; nhóm không phải terminal chỉ có 57 từ. Sai số của mọi tỉ lệ khoảng vài điểm phần trăm.
- Log chứa cả từ đang thử nghiệm IME (tên lệnh, từ gõ để thử Telex), nên EN và va chạm có thể lệch.
- Terminal phần lớn là chat với một CLI bằng tiếng Việt, không phải lệnh shell; flavour terminal của generator vì vậy gần chat hơn tên gọi.
- Số `raw = rendering` của synthetic thấp hơn thật vì log thật có dấu câu và ký hiệu mà generator chỉ sinh trong flavour terminal/IDE.
