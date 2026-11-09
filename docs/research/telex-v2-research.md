# Flexible Telex v2 research

> **Historical research note** · **Date:** 2026-11-08 · **Still current:** yes, as the rationale for `crates/nimk-telex`; the live rules are in [10](../10-telex.md) and the 110-row conformance table is `crates/nimk-telex/tests/conformance.rs` · **Code:** `crates/nimk-telex/src`

Sources: fcitx5-unikey `ukengine.cpp`, bamboo-core, OpenKey `Engine.cpp`. Evidence labels: **B** = asserted in bamboo's tests; **U** and **O** = read from Unikey/OpenKey source, never executed (so verify before freezing tests); **N** = NeuralIMK's own choice. Apple's built-in Vietnamese input is unverified; no Apple-specific rules are encoded.

## 1. Findings that drive the design

- All three engines model a word as (onset, vowel cluster, coda) + marks + one tone, and re-place the tone on every new letter (B: `chuyr` then `+een` gives `chuyển`). So the tone is stored on the syllable and rendered, not stored on a character.
- Unikey "free marking" only removes the rule that the mark target must be the last typed char. Tone keys are always allowed once a vowel cluster exists.
- Unikey applies marks to the last vowel cluster. Bamboo scans backwards and applies a mark only if the result is valid (CV/VC matrices), which is a better gate.
- Once the buffer is not a valid Vietnamese prefix, Unikey and OpenKey stop transforming for the rest of the word.
- Auto-restore differs: Unikey restores raw keys at word end if the word is non-Vietnamese; OpenKey only if a vowel carries a mark; Bamboo never destroys and exposes both VI and raw strings (B: `noww` gives VI `now`, raw `noww`).
- `w`: try the hook first, else emit `ư`. `ww` gives `w`. `[` gives `ơ`, `]` gives `ư`.
- `ưo` completes to `ươ` when a valid coda or `i`/`u` follows (B: `duwoi` gives `dươi`). With nothing after, and onset h/kh/th/qu, `uo`+`w` gives `uơ` (B: `Thuow` gives `Thuơ`).
- Late `dd`: Unikey converts only the initial `d`, with no validity check (`dungd` gives `đung`); Bamboo converts the rightmost valid `d`.

## 2. Rules (reference: Unikey modern style)

- R1. Parse raw keys into a syllable model and re-render after every key. Output = best VI candidate + `valid` flag + raw string; never delete raw.
- R2. Tone keys `s f r x j` apply to the current syllable if it has a vowel cluster, wherever the key comes after. No vowel yet means literal. The same tone again removes it, appends the key literally and locks the word (R7). A different tone replaces. `z` removes the tone.
- R3. Coda `c/ch/p/t` accepts only `s` and `j`. Other tones are literal and the candidate is invalid (B: `catr` stays `catr`).
- R4. Tone target (modern style, N). A marked vowel wins. One vowel: it. Two vowels: `oa`, `oe`, `uy` take the second (`hoà`, `thuỷ`, `quý`); other pairs without coda take the first (`mía`, `múa`); with a coda, the second (`toán`). Three vowels: the middle. Modern style keeps the tone in place when a coda is appended (`hoà` to `hoàn`). An `old_style` option (`hòa`, `thủy`) exists.
- R5. `gi` and `qu` are consonants when another vowel follows (`giá`, `quả`). `gin` keeps `i` as the nucleus-less onset (`gìn`).
- R6. Shape keys:
  - `aa/ee/oo` apply to the last cluster's matching plain vowel, even with consonants between (`theme` gives `thêm`), only if the result is valid.
  - `w` hooks the last cluster (`a` to `ă`, `o` to `ơ`, `u` to `ư`, `uo` to `ươ`), also late (`tuongw`). Otherwise with no vowel yet it gives `ư` (`tw` gives `tư`). After `e`, `i`, `y` it is literal.
  - `dd` converts only the syllable's initial `d`, anywhere after it, if the result is valid (`dungd` gives `đung`). A word starting with `dd` is always allowed.
- R7. Undo: repeating an effect key reverts that transform and appends the key literally (`aaa` gives `aa`, `ww` gives `w`). After any undo, lock the rest of the word as raw.
- R8. Validity: onset, vowel cluster (Bamboo's `vowelSeqs`), coda in {c ch m n ng nh p t}, CV/VC compatibility, and R3. If invalid, keep the best VI string but set `valid = false`; the model decides. Stop-after-non-Vietnamese applies only to shape keys, never to raw.
- R9. Case follows the base letter; effect keys are case-insensitive (`VIEETJ` gives `VIỆT`).

## 3. Conformance examples

| Keys | Output | Valid | Src |
| --- | --- | --- | --- |
| hoaf / hoafn | hoà / hoàn | yes | U |
| quys | quý | yes | U |
| giuwx | giữ | yes | U |
| loanj | loạn | yes | B |
| theme | thêm | yes | U |
| nguowif | người | yes | B |
| duocwj | dược | yes | U |
| dungd | đung | yes | U |
| catr | catr | no | B |
| aaa | aa | no | U |
| ww | w | yes | B |
| window | ưindow (raw window) | no | U |
| class | class | no | U |

## 4. Inverse problem (for synthetic data)

These are heuristics, not measured. Per syllable the generator samples:

- Tone position: about 60% at syllable end, 30% right after the nucleus, 10% after the vowel only; rare out-of-order cases at 3%.
- Hat marks `aa/ee/oo`: 90% immediately, 8% later in the word, 2% after the tone.
- `w`: 70% immediately after the vowel; about 25% last for `ươ` words; `[` and `]` about 1%.
- `dd`: 92% immediate, 6% after the nucleus/coda, 2% word-final.
- Undo patterns about 1%; caps: lowercase 85%, Title 12%, ALL CAPS 3%.
- Negative set: English tokens containing the Telex keys (`window`, `class`, `status`, `case`, `when`, `with`); the Telex tool must still output raw plus an invalid VI candidate.
