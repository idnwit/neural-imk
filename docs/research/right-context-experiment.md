# Thí nghiệm: giá trị của ngữ cảnh phải (từ kế tiếp)

> **Trạng thái:** draft · **Cập nhật:** 2026-11-09 · **Code:** `crates/nimk-synth/examples/right_context.rs`

Trên dữ liệu synthetic hiện tại, thêm phím của từ kế tiếp không cải thiện quyết định ngôn ngữ cho từ pending. GRU left+right đạt ambiguous 0.975 so với 0.976 của left-only. Cận trên Bayes cũng chỉ tăng 0.0 điểm ambiguous và +0.3 điểm trên nhóm phím trùng. Khuyến nghị: **không** làm UX cửa sổ 2 từ dựa trên số đo này.

## Câu hỏi

Nếu giữ 2 từ pending, model thấy thêm phím của từ kế tiếp khi chốt từ trước. Câu hỏi là quyết định VI/EN của từ pending tốt lên bao nhiêu so với hôm nay (chỉ ngữ cảnh trái). Ngưỡng người dùng đặt ra là tăng ít nhất 1.5–2 điểm trên từ mập mờ.

## Phương pháp

Dữ liệu sinh bằng `generate` với `seed` 42, 200 session x 500 từ, `MixProfile` mặc định. 20 script cuối giữ lại để đánh giá, giống hệt `nimk-synth pretrain` (10 000 từ). Mỗi từ là một ví dụ; nhãn là ngôn ngữ của từ.

| Biến thể | Token | Scalar |
| --- | --- | --- |
| `word` | `SEP raw boundary` | cờ Telex của từ |
| `left` (như hôm nay) | 24 token cuối của `lịch sử ++ SEP ++ raw ++ boundary` | cờ Telex + `prev_langs` (nhãn thật khi train, quyết định của chính model khi eval) |
| `left+right` | `left` ++ `RSEP` ++ phím tới hết từ kế tiếp (≤ 16 token) | thêm cờ Telex của từ kế tiếp |
| `right` | `word` ++ phần phải | cờ Telex hai từ |

Mọi biến thể dùng cùng một mạng GRU kiểu `GruAction` (Embedding 32, GRU 64, MLP 64, một đầu render). Train 3 epoch, Adam 3e-3, batch 128, thay 10% token bằng `OTHER`; 2 seed khởi tạo mỗi biến thể. Mạng đặt trong `examples/`, không đụng engine, model đóng gói hay API công khai.

Cận trên Bayes tính chính xác từ xác suất của generator: `P(raw | lang)` từ corpus (gồm biến thể gõ), câu có ngôn ngữ chính với xác suất 0.7, mỗi từ đổi ngôn ngữ độc lập. Ngữ cảnh Bayes giới hạn trong câu vì generator không liên kết giữa các câu.

Ba tập đo:

1. **overall**: mọi từ.
2. **ambiguous**: định nghĩa của eval (`text != raw` và là âm tiết hợp lệ), 6968 từ.
3. **collision**: phím mà generator có thể sinh cho cả hai ngôn ngữ (98 chuỗi: `in`, `an`, `me`, `no`, `ten`, `hang`, `theme`, `to`, `has`, `this`...), 623 từ; 575 từ trong đó cũng là ambiguous.

## Kết quả

Kết quả GRU là trung bình 2 seed; chênh giữa 2 seed trên collision tới 4–5 điểm, trên ambiguous khoảng 0.1–0.5 điểm.

| Phương pháp | overall | ambiguous | collision | collision, từ kế cùng câu |
| --- | --- | --- | --- | --- |
| Bayes, chỉ từ | 0.9862 | 0.9836 | 0.7785 | 0.7915 |
| Bayes, trái (cả phần đầu câu) | 0.9887 | 0.9849 | 0.8186 | 0.8284 |
| Bayes, trái + 1 từ phải | 0.9889 | 0.9849 | 0.8218 | 0.8321 |
| Bayes, cả câu | 0.9894 | 0.9854 | 0.8299 | 0.8413 |
| GRU `word` | 0.9820 | 0.9784 | 0.756 | 0.763 |
| GRU `left` | 0.9810 | 0.9762 | 0.791 | 0.796 |
| GRU `left+right` | 0.9792 | 0.9746 | 0.795 | 0.804 |
| GRU `right` | 0.9791 | 0.9744 | 0.778 | 0.793 |

Nhầm lẫn (lỗi VI→EN / EN→VI, run 0): Bayes chỉ từ 114/24, Bayes trái 63/50, Bayes trái+phải 56/55; GRU `left` 123/59, GRU `left+right` 105/100. Ngữ cảnh phải chỉ dời lỗi từ VI→EN sang EN→VI, tổng gần như không đổi.

Từ thăm dò (n nhỏ, 2–7 lần mỗi từ, chỉ để minh hoạ): `in` 0.83 → 0.83, `an` 0.80 → 0.60, `ten` 1.00 → 1.00, `theme` 1.00 → 0.71 khi chuyển từ GRU `left` sang `left+right`. Bayes trái + phải sửa được `in` (0.83 → 1.00) nhưng làm sai `an` (0.80 → 0.60).

## Vì sao ngữ cảnh phải gần như vô ích ở đây

Generator trộn ngôn ngữ theo kiểu "ngôn ngữ chính của câu + đổi từng từ độc lập (15% EN trong câu VI, 5% VI trong câu EN)". Ngữ cảnh chỉ tiết lộ ngôn ngữ chính của câu, và vài từ bên trái đã đủ cho việc đó. Một từ EN chèn vào câu VI luôn bị kẹp giữa các từ VI, nên từ kế tiếp không bao giờ báo được rằng có code-switch.

```text
generator:  VI VI [EN] VI VI      <- tu chen doc lap, hang xom khong noi gi ve no
thuc te:    VI VI [EN EN EN] VI   <- code-switch thanh cum, tu ke tiep xac nhan cum
```

Sơ đồ trên cho thấy vì sao số đo synthetic đánh giá thấp giá trị của ngữ cảnh phải.

Các thiên lệch khác của dữ liệu synthetic:

| Thiên lệch | Ảnh hưởng |
| --- | --- |
| Không có cụm từ / collocation (`dark theme`, `in the`, `thêm vào`) | Ngữ cảnh phải trong văn bản thật mang nhiều thông tin hơn hẳn |
| Từ EN rút đều từ 1672 + 245 từ, không theo tần suất | Từ chức năng hay gặp (`to`, `in`, `a`, `of`, `as`, `so`, `do`, `no`, `this`) bị lấy mẫu ít; trong văn bản thật collision chiếm tỉ lệ lớn hơn nhiều so với 6.2% |
| Ngôn ngữ giữa các câu độc lập | Ngữ cảnh qua ranh giới câu vô dụng trong synthetic, có ích trong thực tế |
| Từ vựng đóng, đánh giá trên cùng corpus | Phần lớn từ ambiguous giải được chỉ từ chính nó (Bayes chỉ từ 0.984); lỗi còn lại tập trung ở collision |

## Thử giả định: code-switch thành cụm

Để ước lượng giá trị trong văn bản có cụm, thí nghiệm sinh thêm chuỗi 200 000 từ theo xích Markov 2 trạng thái (61% VI, cùng corpus và biến thể). `q` là hệ số tốc độ chuyển: `q = 1` là từ độc lập, `q` nhỏ là cụm dài. Số đo là độ chính xác Bayes trên từ collision.

| `q` | Tỉ lệ hai từ liền nhau cùng ngôn ngữ | trái (filter) | trái + 1 từ phải | cả chuỗi |
| --- | --- | --- | --- | --- |
| 1 | 0.523 | 0.786 | 0.786 | 0.786 |
| 0.6 | 0.714 | 0.798 | 0.816 | 0.816 |
| 0.3 | 0.858 | 0.849 | 0.923 | 0.924 |
| 0.15 | 0.929 | 0.914 | 0.964 | 0.967 |
| 0.05 | 0.976 | 0.968 | 0.988 | 0.990 |

Khi code-switch thành cụm, ngữ cảnh phải tăng 5–7 điểm trên từ collision, và một từ phải đã lấy gần hết lợi ích. Quy ra tập ambiguous (collision chiếm 8.3% từ ambiguous ở phân phối synthetic) chỉ khoảng +0.4–0.6 điểm, vẫn dưới ngưỡng 1.5–2 điểm. Ngưỡng chỉ có thể đạt nếu trong văn bản thật collision chiếm khoảng 25–30% từ ambiguous, điều mà tần suất từ chức năng tiếng Anh khiến không thể loại trừ.

## Độ trễ

Chấm lại từ pending tốn thêm một lần `score()` mỗi từ (khi phím ranh giới của từ kế tiếp đến), không phải mỗi phím. Một hàng 40 token đo được trung vị 261 µs, p99 280 µs (M4 Pro, release), so với khoảng 166 µs của cửa sổ 24 token hôm nay; chi phí GRU tăng tuyến tính theo số token. Nếu muốn cập nhật hiển thị từ pending ở mọi phím của từ kế tiếp thì là thêm một `score()` mỗi phím, khoảng 0.43 ms tổng, vẫn dưới ngân sách p99 2 ms. Chi phí thật là UX: marked text dài 2 từ, ảnh hưởng tương thích app ([09](../09-app-compat.md)) và độ trễ thấy chữ cuối cùng.

## Khuyến nghị

1. Không làm UX cửa sổ 2 từ lúc này. Trên synthetic, lợi ích đo được là 0 (GRU) và tối đa +0.3 điểm collision (Bayes), xa dưới ngưỡng.
2. Câu hỏi chưa được trả lời cho văn bản thật. Synthetic không thể đo vì cấu trúc trộn của generator; số đo giả định cụm cho thấy lợi ích chỉ đạt ngưỡng nếu collision dày đặc.
3. Trước khi xem lại: đo tỉ lệ collision và độ dài cụm code-switch trên log thật (`nimk-replay`), hoặc thêm vào generator chế độ cụm và tần suất từ thực tế. Có thể dùng lại thí nghiệm này với dữ liệu mới.
4. Thay thế rẻ hơn: sửa ngược từ đã commit bằng nháy Shift hai lần (ADR 0008) vẫn đủ cho số ít collision còn sai.

## Cách chạy lại

```sh
cargo run --release -p nimk-synth --example right_context -- --sessions 200 --words 500 --epochs 3 --runs 2
# chỉ cận trên Bayes và thử giả định cụm (vài giây):
cargo run --release -p nimk-synth --example right_context -- --runs 0
```

Mỗi lần train khoảng 40 s; toàn bộ 8 lần khoảng 6 phút.

## Chạy lại trên profile thực tế (2026-11-09)

Cùng ví dụ, `MixProfile::default()` (run, cụm, Zipf; chỉ session chat), 200 x 500 từ, 2 seed GRU. Ambiguous: `word` 0.981 / 0.984, `left` 0.978 / 0.979, `left+right` 0.980 / 0.977, `right` 0.979 / 0.977; collision khoảng 0.88 ở mọi biến thể. Ngữ cảnh phải vẫn không cải thiện (chênh trong nhiễu giữa seed, 0.2–0.5 điểm), kể cả khi code-switch đã thành cụm: GRU 3 epoch không khai thác được cụm tốt như cận Bayes của giả định `q` nhỏ ở trên. Các hàng Bayes in ra dựa trên mô hình generator cũ và không còn đúng; `--legacy 1` chạy lại profile cũ. Kết luận không đổi: chưa làm UX 2 từ dựa trên số đo synthetic.
