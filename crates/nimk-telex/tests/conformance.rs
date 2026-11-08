//! Flexible Telex v2 conformance table (docs/10-telex.md, rules R1-R10).
//!
//! Source tags: B = asserted in bamboo-core `bamboo_test.go`; U / O = read
//! from the Unikey / OpenKey sources (not executed). Rows are `(#, keys,
//! output, valid, tag)`. Rows where this crate deliberately differs from the
//! spec table are in [`DEVIATIONS`] with the reason; the spec row is kept in a
//! comment.

use nimk_telex::{ToneStyle, transform, transform_with};

type Row = (u32, &'static str, &'static str, bool, &'static str);

const MODERN: &[Row] = &[
    (1, "hoaf", "hoà", true, "U"),
    (2, "hoas", "hoá", true, "U"),
    (3, "hoafn", "hoàn", true, "U"),
    (4, "hoanf", "hoàn", true, "U"),
    (6, "thuyr", "thuỷ", true, "U"),
    (8, "quys", "quý", true, "U"),
    (9, "quar", "quả", true, "U"),
    (10, "quas", "quá", true, "U"),
    (11, "quoocs", "quốc", true, "U"),
    (12, "quocos", "quốc", true, "U"),
    (13, "quyeenf", "quyền", true, "U"),
    (14, "gif", "gì", true, "U"),
    (15, "gifn", "gìn", true, "U"),
    (16, "gisa", "giá", true, "B"),
    (17, "giuwx", "giữ", true, "U"),
    (18, "giuwax", "giữa", true, "U"),
    (19, "giamaf", "giầm", true, "B"),
    (20, "khuyas", "khuýa", true, "U"),
    (21, "huyf", "huỳ", true, "U"),
    (22, "khuyeenr", "khuyển", true, "B"),
    (23, "chuyreen", "chuyển", true, "B"),
    (24, "mias", "mía", true, "U"),
    (25, "muaf", "mùa", true, "U"),
    (26, "hais", "hái", true, "U"),
    (27, "taof", "tào", true, "U"),
    (28, "meos", "méo", true, "U"),
    (29, "liuf", "lìu", true, "U"),
    (30, "muis", "múi", true, "U"),
    (31, "toans", "toán", true, "U"),
    (32, "loanj", "loạn", true, "B"),
    (33, "ngoaij", "ngoại", true, "U"),
    (34, "chuaarn", "chuẩn", true, "B"),
    (35, "tuaanf", "tuần", true, "U"),
    (36, "vieetj", "việt", true, "B"),
    (37, "tieengs", "tiếng", true, "U"),
    (38, "tieesng", "tiếng", true, "U"),
    (39, "nghieeng", "nghiêng", true, "U"),
    (40, "bieecs", "biếc", true, "B"),
    (41, "mootj", "một", true, "B"),
    (42, "HIEEUR", "HIỂU", true, "B"),
    (43, "uese", "uế", true, "B"),
    (44, "choas", "choá", true, "B"),
    (45, "nguowif", "người", true, "B"),
    (46, "wowfi", "ười", true, "B"),
    (47, "nuwowcs", "nước", true, "U"),
    (48, "nuocws", "nước", true, "U"),
    (49, "nuocwj", "nược", true, "U"),
    (50, "bacj", "bạc", true, "U"),
    (51, "bacr", "bacr", false, "U"),
    (52, "catr", "catr", false, "B"),
    (53, "toanfz", "toan", true, "U"),
    (54, "toanfs", "toán", true, "U"),
    (55, "az", "az", false, "U"),
    (56, "xin", "xin", true, "U"),
    (57, "theme", "thêm", true, "U"),
    (58, "toio", "tôi", true, "U"),
    (59, "nguoio", "nguôi", true, "U"),
    (60, "tooss", "tôs", false, "B"),
    (61, "tosos", "tôs", false, "B"),
    (62, "aaa", "aa", false, "U"),
    (64, "aww", "aw", false, "U"),
    (65, "ow", "ơ", true, "U"),
    (66, "oww", "ow", false, "U"),
    (67, "uw", "ư", true, "U"),
    (68, "uww", "uw", false, "U"),
    (69, "w", "ư", true, "U/O"),
    (71, "www", "ww", false, "N"),
    (72, "tw", "tư", true, "U"),
    (73, "tuongw", "tương", true, "U"),
    (74, "tuwong", "tương", true, "U"),
    (75, "tuwowng", "tương", true, "B"),
    (76, "nguoiw", "ngươi", true, "B"),
    (77, "duwoi", "dươi", true, "B"),
    (78, "thuow", "thuơ", true, "B"),
    (79, "huow", "huơ", true, "U"),
    (80, "huwowu", "hươu", true, "U"),
    (81, "huowu", "hươu", true, "U"),
    (82, "duwongw", "duongw", false, "B"),
    (83, "duocwj", "dược", true, "U"),
    (84, "dduwowcj", "được", true, "B"),
    (85, "dduocwj", "được", true, "U"),
    (87, "ddd", "dd", false, "U"),
    (88, "ddafi", "đài", true, "B"),
    (89, "dungd", "đung", true, "U"),
    (90, "dad", "đa", true, "U"),
    (91, "dads", "đá", true, "U"),
    (94, "Thuow", "Thuơ", true, "B"),
    (95, "NGUOIW", "NGƯƠI", true, "B"),
    (96, "VIEETJ", "VIỆT", true, "B"),
    (98, "Duwowcj", "Dược", true, "U"),
    (100, "when", "ưhen", false, "U"),
    (101, "with", "ưith", false, "U"),
    (102, "window", "ưindow", false, "U"),
    (105, "case", "cáe", false, "U"),
    (106, "kimso", "kímo", false, "B"),
    (107, "tnoss", "tnos", false, "B"),
    (108, "toorr", "tôr", false, "B"),
    (109, "giw", "gư", true, "B"),
    (110, "noww", "now", false, "B"),
];

/// Spec rows 5 and 7: old tone style.
const OLD: &[Row] = &[
    (5, "hoaf", "hòa", true, "B"),
    (7, "thuyr", "thủy", true, "U"),
];

/// `(row, keys, spec output, spec valid, actual output, actual valid, why)`.
/// The actual column is regression-locked by `deviations_are_stable`.
type Deviation = (
    u32,
    &'static str,
    &'static str,
    bool,
    &'static str,
    bool,
    &'static str,
);

const DEVIATIONS: &[Deviation] = &[
    (
        63,
        "aw",
        "ă",
        true,
        "ă",
        false,
        "`ă` alone is not a syllable (needs a final); text equals spec",
    ),
    (
        70,
        "ww",
        "w",
        true,
        "w",
        false,
        "`w` is not a Vietnamese syllable; text equals spec",
    ),
    (
        86,
        "dd",
        "đ",
        true,
        "đ",
        false,
        "bare `đ` is not a syllable (abbreviation); text equals spec",
    ),
    (
        97,
        "DD",
        "Đ",
        true,
        "Đ",
        false,
        "as row 86; text equals spec",
    ),
    (
        92,
        "oddp",
        "ođp",
        false,
        "oddp",
        false,
        "R6 converts only the initial d; B converts the rightmost; R6 wins",
    ),
    (
        93,
        "ddawks",
        "đắk",
        true,
        "đắk",
        false,
        "`k` is not a Vietnamese final (Đắk Lắk is a name); text equals spec",
    ),
    (
        99,
        "wow",
        "ươ",
        true,
        "ươ",
        false,
        "`ươ` needs a final to be a syllable; text equals spec",
    ),
    (
        103,
        "class",
        "class",
        false,
        "clas",
        false,
        "U: tone literal because `cl` is no onset; B row 107 (tnoss -> tnos) applies the tone first; R8 says do not stop tones, B wins",
    ),
    (
        104,
        "status",
        "status",
        false,
        "status",
        false,
        "second vowel run: tone literal (matches spec)",
    ),
];

#[track_caller]
fn check(row: &Row, style: ToneStyle) {
    let &(n, keys, text, valid, tag) = row;
    let t = transform_with(keys, style);
    assert_eq!(
        (t.text.as_str(), t.valid_vi),
        (text, valid),
        "row {n} [{tag}] keys {keys}"
    );
    assert_eq!(t.has_telex, t.text != keys, "row {n}");
    assert!(!t.valid_vi || t.viable_vi, "row {n}: valid implies viable");
}

#[test]
fn modern_style_rows() {
    for row in MODERN {
        check(row, ToneStyle::Modern);
    }
}

#[test]
fn old_style_rows() {
    for row in OLD {
        check(row, ToneStyle::Old);
    }
}

#[test]
fn deviations_are_stable() {
    for &(n, keys, spec, _, actual, valid, why) in DEVIATIONS {
        let t = transform(keys);
        assert_eq!(
            (t.text.as_str(), t.valid_vi),
            (actual, valid),
            "row {n}: {why}"
        );
        let _ = spec;
    }
}

#[test]
fn table_covers_all_110_rows() {
    let mut seen: Vec<u32> = MODERN
        .iter()
        .map(|r| r.0)
        .chain(OLD.iter().map(|r| r.0))
        .chain(DEVIATIONS.iter().map(|d| d.0))
        .collect();
    seen.sort_unstable();
    assert_eq!(seen, (1..=110).collect::<Vec<_>>());
}

/// The v1 verified table of docs/10 (all unchanged in v2).
#[test]
fn v1_verified_table_still_holds() {
    let rows = [
        ("theme", "thêm", true),
        ("theem", "thêm", true),
        ("vietej", "việt", true),
        ("viejet", "việt", true),
        ("nguoiwf", "người", true),
        ("congoj", "cộng", true),
        ("hoaf", "hoà", true),
        ("w", "ư", true),
        ("when", "ưhen", false),
        ("string", "string", false),
        ("tieengs", "tiếng", true),
        ("tiengse", "tiếng", true),
        ("tieesng", "tiếng", true),
        ("tuongw", "tương", true),
        ("giuwax", "giữa", true),
        ("xinsh", "xính", true),
        ("quaf", "quà", true),
        ("nhwng", "nhưng", true),
    ];
    for (keys, text, valid) in rows {
        let t = transform(keys);
        assert_eq!((t.text.as_str(), t.valid_vi), (text, valid), "{keys}");
    }
}
