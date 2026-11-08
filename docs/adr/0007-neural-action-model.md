# 0007. Neural action model thay cho luật cứng

> **Trạng thái:** accepted · **Ngày:** 2026-11-08 · **Supersedes:** một phần 0005 (bỏ hard rules)

## Bối cảnh

Engine trước ADR này dùng luật cứng: `NoTelex`, `InvalidVi` (giữa từ dùng `viable_vi`, lúc commit dùng `valid_vi`). Composer cũng tự quyết khi nào append, pop hay commit. Model chỉ trả `p(VI)` cho phần còn lại.

Yêu cầu mới: mọi quyết định phải do neural network đưa ra. Ta chỉ quy định contract: event vào, buffer, rồi model dự đoán và biến đổi buffer hoặc xuất output.

## Quyết định

Model là một **policy chọn action**. Engine chỉ thực thi action một cách cơ học.

```text
 KeyEvent --+
            v
 +---------------------+   Observation    +-----------------+
 | Engine (mechanical) | ---------------> | ImeModel (NN)   |
 |  buffer: raw keys   |                  |  edit head  (4) |
 |  telex tool: vi     | <--------------- |  render head(1) |
 +---------------------+      Action      +-----------------+
            |
            v
 EngineOutput: marked text / commit text / pass key to app
```

Engine chỉ thực thi action; mọi lựa chọn ngôn ngữ và biên từ nằm trong model.

**Observation** gồm:
- event hiện tại và cửa sổ W event gần nhất;
- buffer: `raw`, bản Telex `vi`, các cờ `valid_vi` / `viable_vi` (là feature, không phải luật);
- bản đang hiển thị, mode trước đó;
- `prev_langs`, app context, thời gian từ lần switch cuối.

**Action** gồm hai đầu ra:

| Head | Lớp | Engine làm gì |
| --- | --- | --- |
| edit | `Append` | Thêm phím vào `raw` |
| edit | `Pop` | Bỏ phím cuối của `raw` |
| edit | `Commit` | Commit bản đang hiển thị, trả phím cho app |
| edit | `Pass` | Trả phím cho app, buffer giữ nguyên |
| render | `Vi` / `En` | Hiển thị/commit `vi` hay `raw` |

**Action mask (cấu trúc, không phải luật ngôn ngữ).** Engine chỉ chặn action vô nghĩa về mặt vật lý: `Append` với phím không in được, `Pop` khi buffer rỗng. Action bị chặn thì rơi về `Pass`. Model không bao giờ được hỏi để quyết định hotkey: hotkey là kênh dạy, thuộc tầng I/O.

**Telex là công cụ, không phải luật.** Hàm `nimk_telex::transform` tất định sinh ra bản `vi` và các cờ. Model tự quyết có dùng bản đó hay không.

**Học.**
- Render head học online từ nhãn: correction có trọng số 1.0, implicit có trọng số nhỏ. Nhãn của một từ áp cho mọi bước của từ đó.
- Edit head được pre-train bằng dữ liệu tổng hợp (behaviour cloning từ script: chữ thì `Append`, biên từ thì `Commit`, backspace thì `Pop`). Online learning cho edit head **tắt mặc định**, vì chưa có tín hiệu sửa lỗi đáng tin.
- Ví dụ huấn luyện mang `edit_weight` và `render_weight` riêng.

**Manual switch giữa từ.** Đây là lệnh của người dùng, không phải luật ngôn ngữ. Render bị khoá theo lựa chọn của người dùng cho tới hết từ.

**Backend NN.** `burn` 0.22.1 backend `flex` cho `GruAction`, chọn sau khảo sát ([ADR 0009](0009-burn-nn-backend.md)); `LinearAction` viết tay làm đường cơ sở. Model nằm sau trait `ImeModel`/`ImeLearner`, backend đổi được.

**Hysteresis.** Chỉ làm ổn định hiển thị giữa từ. Tại bước `Commit`, rendering là quyết định mới của model; hotkey giữa từ khoá rendering đến hết từ.

## Hệ quả

- Engine đơn giản hơn: không còn `DecisionRule` trong luồng quyết định. Trường `rule` của sự kiện `Decision` được giữ để đọc log cũ, log mới ghi `null`.
- Chất lượng ngày đầu phụ thuộc vào pre-train tổng hợp. Thiếu pre-train thì model trung tính gõ sai, nên IME phải đi kèm snapshot đã pre-train.
- Rủi ro: edit head sai thì làm hỏng việc gõ, ví dụ chữ bị `Pass`. Giảm rủi ro bằng mask, pre-train, và metric "edit accuracy" trong replay.
- Latency: model per-key phải giữ p99 < 1 ms. Benchmark backend là điều kiện chấp nhận; đã đạt (xem Kết quả).

## Kết quả

Số liệu từ lần tuning 2026-11-08, seed 42; chi tiết và cách đo ở [04](../04-engine-ml.md#kết-quả-đo).

| Model | render@commit (held-out) | amb EN | Replay online (log synthetic seed 7) | Replay `--frozen` |
| --- | --- | --- | --- | --- |
| `LinearAction` 200 x 500 x 2 | 0.975 | 0.944 | 97.5% | — |
| `GruAction` 200 x 500 x 2 (102 s) | 0.975 | 0.909 | 97.3%, đủ 10 000 từ | 97.4% |

- Độ chính xác edit đạt 1.000; độ trễ `score` p99 là 0.5 µs (`LinearAction`) và khoảng 170 µs (`GruAction`), dưới ngưỡng 1 ms.
- Trước tuning, `GruAction` chỉ đạt 0.673 và replay chỉ commit 5746 trên 10 000 từ.
- Bài học rò nhãn (label leak): khi pretrain bằng teacher forcing, feature `shown` (rendering đang hiện) chính là render đích của phím trước, nên model học chép nó và render trên tập giữ lại chỉ đạt khoảng 0.65. Scalar `SHOWN_VI` nay dành riêng và luôn bằng 0; snapshot cũ phải pretrain lại. Mọi feature của observation phải tính được ở thời điểm suy luận từ thông tin không phụ thuộc đích.
- Cập nhật render-only trôi thân GRU, nên `GruAction` chỉ học online từ correction (`learns_from_confirmations() = false`); giả thuyết thêm trọng số theo vị trí phím bị bác bỏ.
- Phần còn lại của sai số là các phím trùng nhau ở hai ngôn ngữ (`in`, `ten`, `an`, `hang`, `me`, `no`), không thể phân biệt nếu thiếu ngữ cảnh.
