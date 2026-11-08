# 0004. Mọi dữ liệu bền đi qua trait `Storage`

> **Trạng thái:** accepted · **Ngày:** 2026-11-08

## Bối cảnh

Log ghi mọi phím người dùng gõ; PoC cố ý để plaintext, nhưng bản sau chắc chắn cần mã hoá.
Replay và test cần chạy trên bộ nhớ, không chạm file thật.

## Quyết định

- Event log, model snapshot, config và replay buffer đi qua `Storage` (`EventSink`, `EventSource`, `BlobStore`) trong `nimk-storage`.
- `Storage` chỉ thấy `&[u8]`; serialize nằm ở lớp trên.
- PoC có `PlainFsStorage` (file thường) và `MemStorage` (test, replay).
- Không crate nào ngoài `nimk-storage` dùng `std::fs` (trừ `nimk-replay` khi ghi report); `arch-lint.toml` hiện chặn `std::fs` trong `nimk-types`, `nimk-telex`, `nimk-policy`, `nimk-engine` và `nimk-ffi`.

## Hệ quả

- Mã hoá hoặc nén là một decorator `EncryptedStorage<S: Storage>`, không sửa engine hay replay.
- Mỗi record/blob là một đơn vị độc lập, thuận cho mã hoá từng phần.
- Thêm một lớp `Box<dyn ...>`; log writer chạy nền nên chi phí này không chạm hot path.
- Truy vấn log phải đọc tuần tự theo segment; không có index.
