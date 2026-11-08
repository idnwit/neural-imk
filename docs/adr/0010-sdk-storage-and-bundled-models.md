# 0010. Vị trí lưu trữ theo SDK và model pretrain đóng gói

> **Trạng thái:** accepted · **Ngày:** 2026-11-08

## Bối cảnh

Bản trước viết cứng `~/Library/Application Support/NeuralIMK` và `~/Library/Logs/NeuralIMK.log` trong Swift, và bắt người dùng copy model pretrain bằng tay.
macOS định nghĩa sẵn các thư mục này qua `FileManager`; app còn có thể ship tài nguyên chỉ đọc trong bundle.

## Quyết định

- Swift lấy data dir từ `FileManager.url(for: .applicationSupportDirectory, in: .userDomainMask, ...)` nối `Bundle.main.bundleIdentifier`, log từ `.libraryDirectory`/`Logs`, model gốc từ `Bundle.main.resourceURL/models`; không còn đường dẫn viết cứng. Diagnostics đồng thời đi qua `os.Logger`.
- `nimk-storage` thêm `LayeredStorage<U, D>` (sự kiện và ghi vào tầng người dùng, đọc rơi xuống tầng mặc định), `ReadOnlyDirBlobs`, `BlobStore::delete` và `BlobStore::origin`.
- `make models` pretrain hai model (seed 42, 200 x 500, 2 epoch) vào `target/models/bundle`; `scripts/build-app.sh` copy vào `Contents/Resources/models`. File sinh ra không commit.
- Engine không đổi logic nạp: `get` tự rơi xuống model gốc. Thêm `checkpoint_source` (`User`/`Bundled`/`None`) cho menu và `reset_model` xoá checkpoint người dùng để quay về model gốc. FFI: `nimk_new(data_dir, models_dir, app_version)`, `nimk_reset_model`, `nimk_checkpoint_source` (thay `nimk_has_checkpoint`).

## Hệ quả

- Lần chạy đầu bắt đầu từ model pretrain, học online, lưu checkpoint đã cải thiện vào Application Support; có thể khôi phục model gốc từ menu.
- App lớn thêm khoảng 0.8 MB; build lần đầu chậm thêm khoảng 100 s do pretrain GRU (cache trong `target/`).
- Đổi `PlainFsStorage` sang vị trí khác chỉ cần đổi chuỗi `data_dir` Swift truyền vào.
- Ngoại lệ: `scripts/install.sh` vẫn dùng `$HOME/Library/Input Methods`, vị trí Apple quy định cho input method.
- Model gốc là cố định theo bản build; model đã học thêm không tự cập nhật khi app nâng cấp, người dùng phải khôi phục.
