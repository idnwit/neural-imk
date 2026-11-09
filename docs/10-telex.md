# Telex

> **Status:** stable · **Updated:** 2026-11-09 · **Code:** `crates/nimk-telex/src`

`nimk-telex` is a custom Telex engine in the free-typing style of Unikey, with modern tone placement (`hoà`, `thuý`).
It is deterministic. It produces a Vietnamese candidate plus validity flags, and the model decides whether to show it ([ADR 0007](adr/0007-neural-action-model.md)).

## Role in the system

```text
 raw keys --> nimk_telex::transform(raw) --> Transformed { text, has_telex, valid_vi, viable_vi }
                                                  |
                                                  v
                          Observation.vi / valid_vi / viable_vi  (model features)
```

Telex never picks the language, and the raw key string is never deleted.

| API | Meaning |
| --- | --- |
| `transform(raw)` | Interpret one word's keys (modern tone placement) |
| `transform_with(raw, ToneStyle)` | Same; `ToneStyle::Old` gives `hòa`, `thủy` |
| `Transformed.text` | Vietnamese candidate; equals `raw` if nothing changed |
| `Transformed.has_telex` | `text != raw` |
| `Transformed.valid_vi` | `text` is a complete valid syllable |
| `Transformed.viable_vi` | `text` is valid, or a prefix of a valid syllable |
| `encode(word)` | Canonical keys that type `word` |
| `encode_variants(word)` | Up to 8 other realistic key orders, each checked to round-trip |
| `is_valid_syllable`, `is_viable_prefix` | Syllable checks |

**Maintenance rule:** models learn from Telex output. Any Telex change needs `make models FORCE=1` to retrain the bundled models.
To teach synthetic data a new typing style, add it to `encode_variants` (`MixProfile::p_variant`).

## Rules

Reference behavior is Unikey in modern mode. `crates/nimk-telex/tests/conformance.rs` tests each rule.

**R1. Syllable model.** A word is (initial, vowel group, final) plus marks and one tone. The tone belongs to the syllable and is recomputed after every key.

**R2. Tone keys `s f r x j`.** They apply anywhere after the vowel group: right after the vowel, after the final, or after a late mark (`toasn`, `toans`, `tieesng`).
With no vowel yet (`xin`) they are plain letters. Repeating the same tone key removes the tone, types the letter, and locks the word (R7). A different tone key replaces the tone.
`z` clears the tone, or is a letter if there is none. A word with two separate vowel groups (`status`) treats tone keys as letters.

**R3. Finals `c ch p t`** take only sắc `s` and nặng `j`. `f r x` there are letters and the candidate is invalid (`bacr`, `catr`).

**R4. Tone position (modern).**
- A marked vowel (â ê ô ơ ư ă) wins: `iê yê uô ươ uyê` put the tone on `ê ô ơ`; `ưa` on `ư`; `uâ` on `â`.
- One vowel: that vowel.
- Two vowels: `oa oe uy` use the second (`hoà`, `thuỷ`, `quý`). Other pairs use the first with no final (`mía`, `múa`, `hái`) and the second with a final (`toán`, `loạn`).
- Three vowels: the middle one (`ngoại`).
- Adding a final never moves the tone (`hoà` to `hoàn`). `ToneStyle::Old` gives `hòa`, `thủy`.

**R5. `gi` and `qu` initials.** They are initials when a vowel follows (`gì`, `giá`, `quả`, `quốc`). With only `gi` plus a final (`gin`), `i` is the vowel (`gìn`).

**R6. Mark keys `aa ee oo`, `w`, `dd`.**
- `aa ee oo` act on the matching plain vowel in the last vowel group, even with a final in between (`theme` gives `thêm`, `toio` gives `tôi`, `quocos` gives `quốc`). They apply only if the result is valid or viable. `ươ`/`uơ` plus `o` gives `uô` (`nguoio` gives `nguôi`).
- `w` adds a hook to the last vowel group: `a` to `ă`, `o` to `ơ`, `u` to `ư`, even late (`tuongw`, `duocw`). `uo` plus `w` gives `ươ`, except after `h kh th qu` with nothing following, which gives `uơ` (`thuow` gives `thuơ`, `thuowr` gives `thuở`).
  With no vowel yet, `w` gives `ư` (`tw` gives `tư`, `giw` gives `gư`). Otherwise `w` is a letter. `ưo` completes to `ươ` once a final or `i/u` follows (`tuwong`, `duwoi`, `huowu`).
- `dd` changes only the first `d` of a syllable, at any time if the result is valid (`dungd` gives `đung`, `dad` gives `đa`). A word that starts with `dd` always works (abbreviations).

**R7. Cancel.** Repeating a key with the same effect undoes it and types the key (`aaa` gives `aa`, `ass` gives `as`, `ww` gives `w`, `tooss` gives `tôs`).
After any cancel, the rest of the word is raw. This avoids transform-then-cancel loops.

**R8. Validity.** A syllable is valid when its initial is in `b c ch d đ g gh gi h k kh l m n ng ngh nh p ph qu r s t th tr v x`, its vowel group is in bamboo's `vowelSeqs` table, its final is in `c ch m n ng nh p t`, the pair is allowed, and the tone obeys R3.
An invalid word still gets the best Vietnamese string with `valid_vi = false`; the model decides. Only mark keys (R6) are blocked by viability, never the raw string.

**R9. Case.** Output keeps the input case. Effect keys ignore case (`VIEETJ` gives `VIỆT`, `DD` gives `Đ`). `w` becoming `ư` takes the case of `w`.

**R10. Mixed case** (`Dd`, `dD`) works: `Duwowcj` gives `Dược`.

## Differences from other engines

| Topic | Unikey / OpenKey / bamboo | NeuralIMK |
| --- | --- | --- |
| `www` | bamboo `wư`; Unikey `ww` | `ww` (R7: cancel locks the word) |
| `oddp` | bamboo `ođp`; Unikey only the first `d` | `oddp` (R6) |
| Tone position | bamboo `hòa`; Unikey modern `hoà` | Modern by default; `ToneStyle::Old` optional |
| `class`, `status` | Unikey: tone key is a letter when the initial is invalid | Tone applies like bamboo (`tnoss` gives `tnos`) unless there are two vowel groups (`status`); `class` gives `clas` |
| Restore at word end | Unikey restores raw; OpenKey only with marks; bamboo keeps both | Telex keeps the raw string; the model picks |
| Lone `w` | bamboo "Telex 2" gives `ư`; plain Telex does not | `ư` (R6) |
| Apple's Telex | Undocumented | Not modeled |

## Test table

All 110 rows are in `crates/nimk-telex/tests/conformance.rs` (`MODERN`, `OLD`, `DEVIATIONS`). Source: **B** = assert in bamboo's `bamboo_test.go`; **U/O** = read from Unikey/OpenKey code, not run; **N** = NeuralIMK decision. A sample:

| Keys | Output | Valid | Src | | Keys | Output | Valid | Src |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `hoaf` | hoà | yes | U | | `theme` | thêm | yes | U |
| `thuyr` | thuỷ | yes | U | | `toio` | tôi | yes | U |
| `gifn` | gìn | yes | U | | `nguoio` | nguôi | yes | U |
| `giamaf` | giầm | yes | B | | `tooss` | tôs | no | B |
| `mias` | mía | yes | U | | `aaa` | aa | no | U |
| `loanj` | loạn | yes | B | | `www` | ww | no | N |
| `tieesng` | tiếng | yes | U | | `tw` | tư | yes | U |
| `nguowif` | người | yes | B | | `tuongw` | tương | yes | U |
| `bacr` | bacr | no | U | | `thuow` | thuơ | yes | B |
| `xin` | xin | yes | U | | `dungd` | đung | yes | U |
| `tnoss` | tnos | no | B | | `VIEETJ` | VIỆT | yes | B |

### Deviations from the research table

`deviations_are_stable` locks these rows. They keep the table's `text` unless noted.

| Keys | Table | Actual | Reason |
| --- | --- | --- | --- |
| `aw`, `ww`, `dd`, `DD`, `wow`, `ddawks` | valid = yes | text correct, `valid_vi` = no | A lone `ă`, `w`, `đ` or `ươ`, or final `k` (Đắk Lắk), is not a full syllable |
| `oddp` | ođp | oddp | R6 changes only the first `d` of a syllable |
| `class` | class | clas | R8 and row `tnoss` win over the Unikey row: the tone applies, then cancels |
| `status` | status | status | Matches, through the two-vowel-group rule |

No row is `#[ignore]`d. U/O rows were never run against real Unikey or OpenKey; confirm them before treating them as fixed.

## Typing orders for training data

`encode_variants` emits these styles, alone or combined, in fixed order, at most 8:

| Style | Examples for `thêm`, `việt`, `đường` |
| --- | --- |
| Tone right after the vowel | `toasn`, `hoafn`, `dduwowfng` |
| Late marks | `theme`, `vietej`, `dduongwf` |
| Lone `w` for initial `ư` | `nhw`, `ddwowngf` |
| Late `d` | `dungd`, `duwowngdf` |

These frequencies are a heuristic, not measured on real logs. Synthetic data uses `MixProfile::p_variant` (default 0.5); see [04](04-engine-ml.md).

## Sources

- Unikey (fcitx5-unikey): `unikey/ukengine.cpp`, `unikey/inputproc.cpp`: https://github.com/fcitx/fcitx5-unikey
- OpenKey: `Engine.cpp`: https://github.com/tuyenvm/OpenKey
- bamboo-core: `bamboo_test.go`, `bamboo_utils.go`, `spelling.go`: https://github.com/BambooEngine/bamboo-core

Related: [04-engine-ml.md](04-engine-ml.md) (Telex features), [ADR 0007](adr/0007-neural-action-model.md).
