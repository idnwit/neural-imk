# Nghiên cứu Telex linh hoạt (v2)

> **Trạng thái:** stable · **Cập nhật:** 2026-11-08 · **Code:** `crates/nimk-telex/src`

Bản nghiên cứu gốc (tiếng Anh) làm căn cứ cho Telex v2. Nguồn: mã nguồn fcitx5-unikey `ukengine.cpp`, bamboo-core, OpenKey `Engine.cpp`. Nhãn U/O là suy ra từ việc đọc mã nguồn, chưa chạy thật; nhãn B là assertion trong test của bamboo. Quy tắc đang áp dụng xem [../10-telex.md](../10-telex.md).

---

# NeuralIMK flexible Telex v2 - research and spec

Evidence key. **B** = asserted in bamboo-core `bamboo_test.go` (github.com/BambooEngine/bamboo-core, `bamboo_test.go`, `bamboo_utils.go` findToneTarget/findMarkTarget/generateTransformations, `spelling.go`). **U** = derived by reading fcitx5-unikey `unikey/ukengine.cpp` (processRoof L1065, processHookWithUO L1209, processHook L1359, getTonePosition L1507, processTone L1533, processDd L1600, processTelexW L1752, processAppend L1870, processWordEnd L2905, lastWordIsNonVn L2955) and `inputproc.cpp` TelexMethodMapping L59. **O** = derived by reading OpenKey `Engine.cpp` (handleMainKey L1055, insertW L871, handleModernMark L629, checkSpelling L176, checkRestoreIfWrongSpelling L1204). **N** = NeuralIMK choice. U/O rows were NOT executed (sandbox refused running third-party code); verify them against a real build before freezing tests. B rows are verbatim test assertions.

Apple built-in: web sources only say Apple ships a UniKey-derived Vietnamese input (Telex, Simple Telex, VNI, VIQR; https://en.wikipedia.org/wiki/UniKey_(software), https://discussions.apple.com/thread/250411576). I found no authoritative description of its tone placement or `w` handling, so it is unverified. Do not encode Apple-specific rules without testing a Mac.

## 1. Findings that drive the design

- All three engines treat the word as **(onset, vowel cluster, coda) + marks + one tone**, and re-place the tone every time a letter arrives (B: `chuyr`->chuy, then `+een` -> chuyển, via refreshLastToneTarget; U: processRoof/processHook move the tone). Do not store the tone on a character; store it on the syllable and render.
- Unikey "free marking" (gõ dấu tự do) only removes the rule "mark target must be the last typed char". Tone keys are always allowed once a vowel cluster exists. Bamboo has the same (EfreeToneMarking).
- Unikey applies marks to the **last vowel cluster** of the current word. Bamboo scans backwards over the current syllable and applies the mark only if the result passes `isValid` (CV/VC matrices in spelling.go). Bamboo's validity gate is better; Unikey's no-gate approach relies on word-end restore.
- Once the buffer is not a valid Vietnamese prefix, Unikey (form `vnw_nonVn`, processTone returns append when `vOffset<0`) and OpenKey (`tempDisableKey`) stop transforming for the rest of the word.
- Auto-restore: Unikey restores raw keystrokes at word end (space/punct) if `lastWordIsNonVn` (nonVn form, incomplete vowel seq, invalid CVC, or c/ch/p/t coda with ` ? ~ tone). Words starting `dd` are exempt (`m_singleMode`). OpenKey restores only if some vowel carries a mark. Bamboo never destroys: it exposes both `GetProcessedString(VietnameseMode)` and `(EnglishMode)` (B: `noww` -> VI "now", raw "noww").
- Unikey Telex `w`: try hook first (processHook); if not applicable, emit ư (mapChar). `ww` -> `w` (B: TestDoubleW). `[`->ơ, `]`->ư. OpenKey: same, except Simple Telex 1. Bamboo "Telex 2" adds standalone `w`->ư; bamboo plain "Telex" does not.
- ưo->ươ completion: Unikey completes when a valid coda or i/u follows (processAppend L1983, L2157); bamboo uses `regUOhTail` virtual horn (B: `duwoi`->dươi, `wowfi`->ười). `uo`+`w` with nothing after, onset h/kh/th/qu, gives `uơ` (B: Thuow->Thuơ; U L1248; O insertW); otherwise ươ.
- Late `dd`: Unikey converts the **initial** consonant `d` (c1) only, no validity check (`dungd`->đung); undo if already đ. Bamboo converts the rightmost valid `d` (B: `oddp`->ođp). 

## 2. Normative rules: flexible Telex v2 (reference: Unikey modern style)

R1 Parse the raw keys into a syllable model; re-render after each key. Output = best VI candidate + `valid` flag + raw string; never delete raw.
R2 Tone keys s f r x j: apply to the current syllable if it has a vowel cluster (anywhere after it: right after nucleus, after coda, after later marks). No vowel yet (`xin`, `rừng`, `sa` initial) -> literal. Same tone again -> remove tone, append key literally, lock word (R7). Different tone -> replace. `z` removes tone; literal if no tone.
R3 Coda c/ch/p/t only accepts s (sắc) and j (nặng); f r x there -> literal key, candidate marked invalid (B: `catr`->catr).
R4 Tone target (modern, N, Unikey `modernStyle`): marked vowel (â ê ô ơ ư ă) wins (iê/yê/uô/ươ/uyê -> ê/ô/ơ; ưa -> ư; uâ -> â). Else 1 vowel: it. 2 vowels: oa, oe, uy -> second (hoà, hoè, thuỷ, quý); other pairs without coda -> first (mía, múa, hái, tào, líu); with coda -> second (toán, loạn). 3 vowels -> middle (oái, ngoại, khuỷu, uyê->ê, ươi->ơ, uôi->ô). Why modern: tone position never changes when a coda is appended (hoà -> hoàn), cheaper for NN-side prediction, matches Bamboo non-std flag (B: `choas`->choá). Provide `old_style` flag (hòa/thủy) as option.
R5 Onsets `gi` and `qu` are consonants when another vowel follows: gì, giữ, giá, quả, quý, quốc. `gi`+coda-only (`gin`) keeps i as the nucleus-less onset: gìn (U cs_gin). Bamboo exception: `g`+`iê`+coda stays g+iê (giếng).
R6 Shape keys a e o (double), w, d:
 - aa/ee/oo: apply to the last cluster's matching plain vowel, even if consonants intervene (theme->thêm), only if result is a valid syllable; uo/uoi/uơ + o -> uô (nguoio->nguôi); ươ + o -> uô (ươ undone to uô, bamboo L"regUhO").
 - w: (1) hook on last cluster: a->ă, o->ơ, u->ư, uo/ưo/uơ->ươ (rules above), also late (tuongw, duocw); (2) else if no vowel yet or only an onset: ư (`w`->ư, `tw`->tư); (3) else (after e,i,y) emit literal; candidate `eư` is invalid so raw wins. After vowels where hook already applied -> undo (R7).
 - dd: only the syllable's initial `d`, anywhere after it, if resulting syllable is valid (`dungd`->đung, `dad`->đa). Word beginning `dd` (`dd`, `ddk`) is always allowed (abbreviations).
R7 Undo: repeating the same effect key reverts only that transform and appends the key literally (aaa->aa, ass->as, ww->w, ddd->dd, uww->uw, tooss->tôs B). After any undo, lock the rest of the word as raw (OpenKey `tempDisableKey`; simpler and avoids ping-pong). `www` -> `ww`. Locked word still gets candidate list with raw.
R8 Validity: syllable valid iff onset in {b c ch d đ g gh gi h k kh l m n ng ngh nh p ph qu r s t th tr v x}, vowel cluster in the Bamboo `vowelSeqs` table, coda in {c ch m n ng nh p t}, CV/VC compatible (bamboo spelling.go matrices) and tone/coda rule R3. If invalid, keep emitting the best VI string but set valid=false; do not stop applying tones (NN decides). Do apply Unikey's "stop transforming once non-VN prefix" only to shape keys, never to the raw string.
R9 Case: result letter case = case of the base letter; effect keys are case-insensitive (B: VIEETJ->VIỆT, NGUOIW->NGƯƠI, SD+D->SĐ). `w`->ư takes case of `w`. ALL CAPS and Title are just per-letter case; caps lock inverts only mapChar keys (U).
R10 Mixed-case effect (`Dd`, `dD`) both convert; `Duwowcj`->Dược.

## 3. Conformance table (raw keys -> output)

Valid = valid VI syllable. All rows modern tone style unless noted.

| # | Keys | Output | Valid | Src |
|---|---|---|---|---|
| 1 | hoaf | hoà | y | U modern; B non-std |
| 2 | hoas | hoá | y | U modern |
| 3 | hoafn | hoàn | y | U |
| 4 | hoanf | hoàn | y | U |
| 5 | hoaf (old style) | hòa | y | B std |
| 6 | thuyr | thuỷ | y | U modern |
| 7 | thuyr (old) | thủy | y | U old |
| 8 | quys | quý | y | U |
| 9 | quar | quả | y | U |
| 10 | quas | quá | y | U |
| 11 | quoocs | quốc | y | U |
| 12 | quocos | quốc | y | U (late roof) |
| 13 | quyeenf | quyền | y | U |
| 14 | gif | gì | y | U |
| 15 | gifn | gìn | y | U |
| 16 | gis+a (gisa) | giá | y | B |
| 17 | giuwx | giữ | y | U |
| 18 | giuwax | giữa | y | U |
| 19 | giamaf | giầm | y | B |
| 20 | khuyas | khuýa | y | U |
| 21 | huyf | huỳ | y | U |
| 22 | khuyeenr | khuyển | y | B tone rule uye->e |
| 23 | chuyr then +een | chuyển | y | B (comment) |
| 24 | mias | mía | y | U |
| 25 | muaf | mùa | y | U |
| 26 | hais | hái | y | U |
| 27 | taof | tào | y | U |
| 28 | meos | méo | y | U |
| 29 | liuf | lìu | y | U |
| 30 | muis | múi | y | U |
| 31 | toans | toán | y | U |
| 32 | loanj | loạn | y | B |
| 33 | ngoaij | ngoại | y | U |
| 34 | chuaarn | chuẩn | y | B |
| 35 | tuaanf | tuần | y | U |
| 36 | vieetj | việt | y | B (VIEETJ) |
| 37 | tieengs | tiếng | y | U |
| 38 | tieesng | tiếng | y | U (tone before coda) |
| 39 | nghieeng | nghiêng | y | U |
| 40 | bieecs | biếc | y | B |
| 41 | mootj | một | y | B |
| 42 | tooi oo HIEEUR | HIỂU (last word) | y | B |
| 43 | uese | uế | y | B |
| 44 | choas (non-std) | choá | y | B |
| 45 | nguowif | người | y | B (wowfi) |
| 46 | wowfi | ười | y | B |
| 47 | nuwowcs | nước | y | U |
| 48 | nuocws | nước | y | U late w |
| 49 | nuocwj | nược | y | U |
| 50 | bacj | bạc | y | U |
| 51 | bacr | bacr | n | U R3 |
| 52 | catr | catr | n | B |
| 53 | toanfz | toan | y | U |
| 54 | toanfs | toán | y | U |
| 55 | az | az | n | U |
| 56 | xin | xin | y | U (x onset literal) |
| 57 | theme | thêm | y | U free marking |
| 58 | toio | tôi | y | U |
| 59 | nguoio | nguôi | y | U |
| 60 | tooss | tôs | n | B |
| 61 | tosos | tôs | n | B (late roof) |
| 62 | aaa | aa | n | U |
| 63 | aw | ă | y | B |
| 64 | aww | aw | n | U |
| 65 | ow | ơ | y | U |
| 66 | oww | ow | n | U |
| 67 | uw | ư | y | U |
| 68 | uww | uw | n | U |
| 69 | w | ư | y | U/O (bamboo Telex2) |
| 70 | ww | w | y | B |
| 71 | www | ww | n | N (bamboo gives wư) |
| 72 | tw | tư | y | U |
| 73 | tuongw | tương | y | U |
| 74 | tuwong | tương | y | U/B-like |
| 75 | tuwowng | tương | y | B (uwow) |
| 76 | nguoiw | ngươi | y | B (NGUOIW) |
| 77 | duwoi | dươi | y | B |
| 78 | thuow | thuơ | y | B |
| 79 | huow | huơ | y | U |
| 80 | huwowu | hươu | y | U |
| 81 | huowu | hươu | y | U (L1983) |
| 82 | duwongw | duongw | n | B-like (duwongwj->duongwj) |
| 83 | duocwj | dược | y | U |
| 84 | dduwowcj | được | y | B-like |
| 85 | dduocwj | được | y | U |
| 86 | dd | đ | y | B |
| 87 | ddd | dd | n | U |
| 88 | ddafi | đài | y | B |
| 89 | dungd | đung | y | U |
| 90 | dad | đa | y | U (also bamboo) |
| 91 | dads | đá | y | U |
| 92 | oddp | ođp | n | B |
| 93 | ddawks | đắk | y | B (Telex2) |
| 94 | Thuow | Thuơ | y | B |
| 95 | NGUOIW | NGƯƠI | y | B |
| 96 | VIEETJ | VIỆT | y | B |
| 97 | DD | Đ | y | B (SD+D->SĐ) |
| 98 | Duwowcj | Dược | y | U |
| 99 | wow | ươ | y | B-like (cannot tell from EN; NN decides) |
| 100 | when | ưhen (raw when) | n | U (restore at word end) |
| 101 | with | ưith (raw with) | n | U |
| 102 | window | ưindow (raw window) | n | U (all engines need restore) |
| 103 | class | class | n | U (cl not valid onset, tone literal) |
| 104 | status | status | n | U |
| 105 | case | cáe (raw case) | n | U |
| 106 | kimso | kímo | n | B |
| 107 | tnoss | tnos | n | B |
| 108 | toorr | tôr | n | B |
| 109 | giw | gư | y | B (valid) |
| 110 | noww | now (raw noww) | n | B |

## 4. Inverse problem: how people type a Vietnamese word (for synthetic data)

Heuristic, not measured from corpora. Sample per syllable:
- Tone position: ~60% at syllable end (after coda, "toans"), ~30% right after nucleus ("toasn", "hoafn"), ~10% after the coda-less vowel only. Rare: tone before an unrelated mark (`quyeenf` vs `quyefen`), include at 3%. Always a single tone key, optionally with typo-correction pattern (wrong tone, then correct tone; `z`).
- Hat marks aa/ee/oo: 90% immediately after the vowel; 8% after coda/next letters (`theme`, `toio`, `quocos`); 2% after tone (`tosos`).
- `w`: 70% immediately after u/o/a; ươ words: `uwow`/`uow` + coda (`duwowc`), `duocw`/`duongw` with `w` last (~25%), `[`/`]` ~1%. Standalone `w`->ư for `ư`-initial words (`wowfi`, `ưng`) ~ rare. `uow`+`w` after h/kh/th/qu gives `uơ`.
- `dd`: 92% immediate, 6% right after nucleus/coda (`dungd`, `dadd`-style), 2% word-final.
- Undo patterns: ~1% double key (`aaa`, `ass`) to type literal English inside VN text, followed by raw rest of word.
- Caps: lowercase 85%, Title 12%, ALL CAPS 3%; effect keys follow the neighbouring letter's case (`VIEETJ`), plus mixed `DDuwowc`.
- Negative set: English tokens containing s f r x j w a e o d keys (window, class, status, case, when, with, exact, fix, www, reset). Label by NN as EN; Telex tool must still output raw + invalid VI candidate.
