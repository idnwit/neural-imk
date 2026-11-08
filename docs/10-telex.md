# Kiểu gõ Telex của NeuralIMK (v2, gõ tự do)

> **Trạng thái:** stable · **Cập nhật:** 2026-11-08 · **Code:** `crates/nimk-telex/src`

NeuralIMK dùng Telex tự viết (không dùng vi-rs), theo kiểu "gõ tự do" của Unikey với cách đặt dấu kiểu mới (`hoà`, `thuý`). Telex là **công cụ tất định**: nó sinh bản tiếng Việt và các cờ cho model. Model NN mới quyết định hiển thị VI hay EN ([ADR 0007](adr/0007-neural-action-model.md)).

v2 thay v1 ở năm điểm: dấu và `dd` gõ muộn được (`theme` → thêm, `dungd` → đung), `ưo` tự hoàn chỉnh thành `ươ`, dấu mũ/móc chỉ áp khi kết quả còn là âm tiết hợp lệ, `uơ` gõ được (`thuở`), và huỷ một dấu thì khoá cả từ ở dạng thô.

## 1. Vai trò trong hệ thống

```text
 raw keys --> nimk_telex::transform(raw) --> Transformed { text, has_telex, valid_vi, viable_vi }
                                                  |
                                                  v
                          Observation.vi / valid_vi / viable_vi  (feature cho model)
```

Telex không quyết định ngôn ngữ, nó chỉ cung cấp ứng viên tiếng Việt và cờ hợp lệ. Chuỗi phím thô không bao giờ bị xoá.

| API | Ý nghĩa |
| --- | --- |
| `transform(raw) -> Transformed` | Diễn giải chuỗi phím của một từ theo Telex (đặt dấu kiểu mới) |
| `transform_with(raw, ToneStyle)` | Như trên, `ToneStyle::Old` đặt dấu kiểu cũ (`hòa`, `thủy`) |
| `Transformed.text` | Bản tiếng Việt; giống `raw` nếu không có biến đổi nào |
| `Transformed.has_telex` | `text != raw` |
| `Transformed.valid_vi` | `text` là một âm tiết tiếng Việt hoàn chỉnh, hợp lệ |
| `Transformed.viable_vi` | `text` hợp lệ, hoặc là tiền tố của một âm tiết hợp lệ (dấu đã gõ phải khớp, chữ chưa có dấu còn thêm dấu được) |
| `encode(word)` | Chuỗi phím chuẩn gõ ra `word` (dấu ngay sau chữ, thanh gõ cuối) |
| `encode_variants(word)` | Tối đa 8 cách gõ thực tế khác, thứ tự cố định, mỗi cách đã kiểm `transform` ra đúng `word` |
| `is_valid_syllable`, `is_viable_prefix` | Kiểm tra âm tiết |

**Quy tắc bảo trì:** model học trên đầu ra của Telex. **Mọi thay đổi Telex phải đi kèm `make models FORCE=1`** để pre-train lại hai model đóng gói trong app. Kiểu gõ mới cho dữ liệu tổng hợp thì thêm vào `encode_variants` (`MixProfile::p_variant`).

## 2. Quy tắc v2 (R1–R10)

Mẫu tham chiếu là Unikey kiểu modern. Mỗi quy tắc có test trong `crates/nimk-telex/tests/conformance.rs`.

**R1. Mô hình âm tiết.** Từ là (phụ âm đầu, cụm nguyên âm, phụ âm cuối) cộng dấu và một thanh. Thanh gắn vào âm tiết chứ không vào ký tự, và được đặt lại sau mỗi phím. Đầu ra là ứng viên VI tốt nhất, cờ hợp lệ và chuỗi thô.

**R2. Phím thanh `s f r x j`.** Áp được ở bất kỳ đâu sau cụm nguyên âm: ngay sau nguyên âm, sau phụ âm cuối, sau dấu gõ muộn (`toasn`, `toans`, `tieesng`). Chưa có nguyên âm (`xin`) thì gõ chữ thật. Gõ lại cùng thanh thì bỏ thanh, gõ phím thật và khoá từ (R7). Thanh khác thì thay thanh. `z` bỏ thanh, nếu chưa có thanh thì là chữ `z`. Nếu từ đã có hai cụm nguyên âm tách nhau (`status`) thì phím thanh là chữ thật.

**R3. Phụ âm cuối `c ch p t`** chỉ nhận sắc `s` và nặng `j`. `f r x` ở đó là chữ thật và ứng viên không hợp lệ (`bacr`, `catr`).

**R4. Vị trí thanh (modern).** Nguyên âm có dấu (â ê ô ơ ư ă) thắng: `iê yê uô ươ uyê` mang thanh ở `ê ô ơ`, `ưa` ở `ư`, `uâ` ở `â`. Một nguyên âm: chính nó. Hai nguyên âm: `oa oe uy` ở chữ thứ hai (`hoà`, `thuỷ`, `quý`); cặp khác không có phụ âm cuối ở chữ đầu (`mía`, `múa`, `hái`, `lìu`); có phụ âm cuối ở chữ thứ hai (`toán`, `loạn`). Ba nguyên âm: chữ giữa (`ngoại`, `khuýa`). Vì vậy thêm phụ âm cuối không làm thanh nhảy chỗ (`hoà` → `hoàn`). Tuỳ chọn `ToneStyle::Old` cho `hòa`, `thủy`.

**R5. Phụ âm đầu `gi` và `qu`** là phụ âm khi sau còn nguyên âm (`gì`, `giữ`, `giá`, `quả`, `quý`, `quốc`). Chỉ có `gi` và phụ âm cuối (`gin`) thì `i` là nguyên âm (`gìn`).

**R6. Phím dạng `a e o` (đôi), `w`, `d`.**

- `aa ee oo` áp cho nguyên âm trơn cùng tên ở cụm nguyên âm cuối, dù có phụ âm cuối chen giữa (`theme` → thêm, `toio` → tôi, `quocos` → quốc), nhưng chỉ khi kết quả là âm tiết hợp lệ hoặc tiền tố khả dĩ. `ươ`, `uơ` + `o` thành `uô` (`nguoio` → nguôi).
- `w`: (1) móc lên cụm nguyên âm cuối, `a`→`ă`, `o`→`ơ`, `u`→`ư`, kể cả gõ muộn (`tuongw`, `duocw`); `uo` + `w` thành `ươ`, trừ khi phụ âm đầu là `h kh th qu` và sau đó chưa có gì thì ra `uơ` (`thuow` → thuơ, `thuowr` → thuở). (2) Chưa có nguyên âm (đầu từ hoặc sau phụ âm đầu): `ư` (`w`, `tw` → tư), `giw` → `gư`. (3) Còn lại là chữ `w` thật. `ưo` tự hoàn chỉnh thành `ươ` ngay khi có phụ âm cuối hoặc `i/u` theo sau (`tuwong`, `duwoi`, `huowu`).
- `dd`: chỉ `d` đầu âm tiết, ở bất kỳ thời điểm nào sau đó nếu kết quả là âm tiết hợp lệ (`dungd` → đung, `dad` → đa). Từ bắt đầu bằng `dd` (`dd`, `ddk`) luôn được (viết tắt).

**R7. Huỷ.** Lặp lại phím cùng hiệu ứng thì huỷ đúng biến đổi đó và gõ phím thật (`aaa` → aa, `ass` → as, `ww` → w, `ddd` → dd, `uww` → uw, `tooss` → tôs). Sau mọi lần huỷ, **phần còn lại của từ là chữ thô** (kiểu `tempDisableKey` của OpenKey): tránh vòng lặp biến đổi rồi huỷ.

**R8. Hợp lệ.** Âm tiết hợp lệ khi phụ âm đầu thuộc `b c ch d đ g gh gi h k kh l m n ng ngh nh p ph qu r s t th tr v x`, cụm nguyên âm thuộc bảng `vowelSeqs` của bamboo, phụ âm cuối thuộc `c ch m n ng nh p t`, tổ hợp đầu/vần hợp lệ và thanh theo R3. Không hợp lệ vẫn sinh chuỗi VI tốt nhất và đặt `valid_vi = false`; thanh không dừng lại (model quyết định). Chỉ phím dấu (R6) bị chặn bởi tính khả dĩ, không bao giờ chuỗi thô.

**R9. Hoa thường.** Chữ kết quả giữ hoa thường của chữ gốc; phím hiệu ứng không phân biệt hoa thường (`VIEETJ` → VIỆT, `NGUOIW` → NGƯƠI, `DD` → Đ). `w` thành `ư` lấy hoa thường của `w`.

**R10. Hoa thường lẫn lộn** (`Dd`, `dD`) đều chuyển được; `Duwowcj` → Dược.

## 3. Quyết định khi các engine khác nhau

| Chủ đề | Unikey / OpenKey / bamboo | NeuralIMK |
| --- | --- | --- |
| `www` | bamboo ra `wư`, Unikey `ww` | `ww` (R7: sau huỷ khoá từ, `w` thứ ba là chữ thật) |
| `oddp` | bamboo `ođp` (đổi `d` phải nhất hợp lệ); Unikey chỉ `d` đầu | `oddp`: chỉ `d` đầu âm tiết (R6) |
| Vị trí thanh | bamboo std `hòa`; Unikey modern `hoà` | modern mặc định, `ToneStyle::Old` tuỳ chọn |
| `class`, `status` | Unikey: thanh là chữ thật nếu onset sai | thanh vẫn áp như bamboo (`tnoss` → tnos), trừ khi có hai cụm nguyên âm (`status`); `class` ra `clas` |
| Khôi phục khi hết từ | Unikey khôi phục chữ thô; OpenKey chỉ khi có dấu; bamboo giữ cả hai | Telex không xoá chuỗi thô; model chọn rendering |
| `w` đứng riêng | bamboo "Telex 2" ra `ư`; Telex thường không | `ư` (R6) |
| Telex của macOS | không có tài liệu chính thức | không mã hoá hành vi riêng của Apple |

## 4. Bảng kiểm thử (đại diện)

Bảng đầy đủ 110 dòng nằm trong `crates/nimk-telex/tests/conformance.rs` (`MODERN`, `OLD`, `DEVIATIONS`). Nguồn: **B** = assert trong `bamboo_test.go`; **U/O** = đọc từ mã Unikey/OpenKey, chưa chạy; **N** = quyết định của NeuralIMK.

| Gõ | Ra | Hợp lệ | Nguồn | | Gõ | Ra | Hợp lệ | Nguồn |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `hoaf` | hoà | có | U | | `theme` | thêm | có | U |
| `hoanf` | hoàn | có | U | | `toio` | tôi | có | U |
| `thuyr` | thuỷ | có | U | | `nguoio` | nguôi | có | U |
| `quys` | quý | có | U | | `quocos` | quốc | có | U |
| `gifn` | gìn | có | U | | `tosos` | tôs | không | B |
| `gisa` | giá | có | B | | `tooss` | tôs | không | B |
| `giuwax` | giữa | có | U | | `aaa` | aa | không | U |
| `giamaf` | giầm | có | B | | `uww` | uw | không | U |
| `mias` | mía | có | U | | `www` | ww | không | N |
| `toans` | toán | có | U | | `w` | ư | có | U/O |
| `loanj` | loạn | có | B | | `tw` | tư | có | U |
| `chuaarn` | chuẩn | có | B | | `giw` | gư | có | B |
| `khuyeenr` | khuyển | có | B | | `tuongw` | tương | có | U |
| `tieesng` | tiếng | có | U | | `tuwong` | tương | có | U |
| `bieecs` | biếc | có | B | | `tuwowng` | tương | có | B |
| `uese` | uế | có | B | | `nuocws` | nước | có | U |
| `choas` | choá | có | B | | `duwoi` | dươi | có | B |
| `nguowif` | người | có | B | | `thuow` | thuơ | có | B |
| `wowfi` | ười | có | B | | `huowu` | hươu | có | U |
| `bacr` | bacr | không | U | | `duwongw` | duongw | không | B |
| `catr` | catr | không | B | | `dduocwj` | được | có | U |
| `toanfz` | toan | có | U | | `dungd` | đung | có | U |
| `az` | az | không | U | | `dad` | đa | có | U |
| `xin` | xin | có | U | | `ddafi` | đài | có | B |
| `kimso` | kímo | không | B | | `NGUOIW` | NGƯƠI | có | B |
| `tnoss` | tnos | không | B | | `VIEETJ` | VIỆT | có | B |
| `noww` | now | không | B | | `when` | ưhen | không | U |

### Điểm lệch khỏi bảng nghiên cứu

Các dòng này giữ nguyên `text` của bảng (trừ khi ghi khác) và được khoá bởi `deviations_are_stable`:

| # | Gõ | Bảng | Thực tế | Lý do |
| --- | --- | --- | --- | --- |
| 63, 70, 86, 97, 99, 93 | `aw`, `ww`, `dd`, `DD`, `wow`, `ddawks` | hợp lệ = có | text đúng, `valid_vi` = không | `ă`, `w`, `đ`, `ươ` đứng một mình, hoặc phụ âm cuối `k` (Đắk Lắk), không phải âm tiết đầy đủ |
| 92 | `oddp` | ođp | oddp | R6 chỉ đổi `d` đầu âm tiết; bamboo đổi `d` phải nhất, R6 thắng |
| 103 | `class` | class | clas | Dòng U (thanh là chữ thật vì `cl` không phải onset) mâu thuẫn dòng B `tnoss` → tnos; theo R8 và B, thanh áp rồi bị huỷ |
| 104 | `status` | status | status | Khớp bảng, nhờ luật hai cụm nguyên âm |

Không dòng nào bị `#[ignore]`. Dòng U/O chưa được chạy trên bản Unikey/OpenKey thật; cần đối chiếu trước khi coi là cố định.

### Khác v1 có chủ đích

- `show` ra `show` (v1: `shơ`): `w` chỉ áp khi kết quả còn khả dĩ.
- `bacr` ra `bacr` (v1: `bảc`): R3.
- Huỷ khoá cả từ (v1 chỉ tắt đúng nhóm phím): `assf` ra `asf` (v1: `à`).
- `toio`, `dungd`, `tuwong`, `thuở` hết nằm trong danh sách thiếu.

Mọi dòng khác của bảng kiểm chứng v1 (`theme`, `vietej`, `nguoiwf`, `tiengse`, `xinsh`, ...) vẫn đúng (`v1_verified_table_still_holds`).

## 5. Thứ tự gõ thực tế cho dữ liệu học

`encode_variants` sinh các kiểu gõ sau (một hoặc nhiều kiểu cùng lúc), theo thứ tự cố định, tối đa 8:

| Kiểu | Ví dụ cho `thêm`, `việt`, `đường` |
| --- | --- |
| Thanh ngay sau nguyên âm | `toasn`, `hoafn`, `dduwowfng` |
| Dấu gõ muộn | `theme`, `vietej`, `dduongwf` |
| `w` đứng riêng cho `ư` đầu | `nhw`, `ddwowngf` |
| `d` gõ muộn | `dungd`, `duwowngdf` |

Nguồn tần suất là heuristic, chưa đo trên log thật. Synthetic dùng `MixProfile::p_variant` (mặc định 0.5), xem [04](04-engine-ml.md). `theme` trong câu Vietnamese vs English: xem mục "Kết quả đo" của 04.

## 6. Nguồn

- Unikey (fcitx5-unikey): `unikey/ukengine.cpp` (processRoof, processHookWithUO, processHook, getTonePosition, processTone, processDd, processTelexW, processAppend, processWordEnd, lastWordIsNonVn), `unikey/inputproc.cpp` (TelexMethodMapping): https://github.com/fcitx/fcitx5-unikey
- OpenKey: `Engine.cpp` (handleMainKey, insertW, handleModernMark, checkSpelling, checkRestoreIfWrongSpelling): https://github.com/tuyenvm/OpenKey
- bamboo-core: `bamboo_test.go`, `bamboo_utils.go`, `spelling.go`: https://github.com/BambooEngine/bamboo-core
- Telex của Apple: chỉ biết là dẫn xuất từ UniKey (https://en.wikipedia.org/wiki/UniKey_(software), https://discussions.apple.com/thread/250411576), chưa kiểm chứng trên máy thật.

Liên quan: [04-engine-ml.md](04-engine-ml.md) (feature từ Telex), [ADR 0007](adr/0007-neural-action-model.md).
