# 0007. Neural action model thay cho luật cứng

> **Trạng thái:** proposed · **Ngày:** 2026-11-08 · **Supersedes:** một phần 0005 (bỏ hard rules)

## Bối cảnh

Engine hiện tại dùng luật cứng: `NoTelex`, `InvalidVi` (giữa từ dùng `viable_vi`, lúc commit dùng `valid_vi`). Composer cũng tự quyết khi nào append, pop hay commit. Model chỉ trả `p(VI)` cho phần còn lại.

Yêu cầu mới: mọi quyết định phải do neural network đưa ra. Ta chỉ quy định contract: event vào, buffer, rồi model dự đoán và biến đổi buffer hoặc xuất output.

## Quyết định

Model là một **policy chọn action**. Engine chỉ thực thi action một cách cơ học.

```text
 KeyEvent --+
            v
 +---------------------+   Observation    +-----------------+
 | Engine (mechanical) | ---------------> | ImeModel (NN)   |
 |  buffer: raw keys   |                  |  edit head  (4) |
 |  telex tool: vi     | <--------------- |  render head(2) |
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

**Backend NN.** Chọn sau khảo sát. Model nằm sau trait, backend đổi được.

## Hệ quả

- Engine đơn giản hơn: không còn `DecisionRule` trong luồng quyết định. Trường `rule` của sự kiện `Decision` được giữ để đọc log cũ, log mới ghi `null`.
- Chất lượng ngày đầu phụ thuộc vào pre-train tổng hợp. Thiếu pre-train thì model trung tính gõ sai, nên IME phải đi kèm snapshot đã pre-train.
- Rủi ro: edit head sai thì làm hỏng việc gõ, ví dụ chữ bị `Pass`. Giảm rủi ro bằng mask, pre-train, và metric "edit accuracy" trong replay.
- Latency: model per-key phải giữ p99 < 1 ms. Benchmark backend là điều kiện chấp nhận.
