use nimk_types::{DecisionCtx, Features, Lang};

use crate::FeatureExtractor;

/// Number of hash buckets for sparse features (2^16).
pub const SPARSE_BUCKETS: u32 = 1 << 16;

/// Length of the dense feature vector produced by [`HashedExtractor`].
pub const DENSE_LEN: usize = 8;

/// Side of the caret grid (`CARET_GRID x CARET_GRID` cells).
const CARET_GRID: u32 = 8;

/// Cell token used when the caret position is unknown (outside `0..64`).
const CELL_UNKNOWN: u8 = 0xFF;

/// Only the first `MAX_RAW_CHARS` characters of `raw` produce n-grams, which
/// bounds the sparse vector at `4 * (MAX_RAW_CHARS + 2) + 3` ids.
const MAX_RAW_CHARS: usize = 32;

/// Longest character n-gram.
const MAX_NGRAM: usize = 4;

/// Namespace salts: each feature group hashes a distinct leading byte so the
/// same text in two groups lands in unrelated buckets.
mod salt {
    pub(super) const NGRAM: u8 = 0x01;
    pub(super) const APP: u8 = 0x02;
    pub(super) const APP_CELL: u8 = 0x03;
    pub(super) const CELL: u8 = 0x04;
}

/// 32-bit FNV-1a; stable across platforms and releases (snapshots depend on it).
#[derive(Clone, Copy)]
struct Fnv(u32);

impl Fnv {
    const OFFSET: u32 = 0x811c_9dc5;
    const PRIME: u32 = 0x0100_0193;

    fn new() -> Self {
        Self(Self::OFFSET)
    }

    fn salted(salt: u8) -> Self {
        Self::new().bytes(&[salt])
    }

    fn bytes(mut self, bytes: &[u8]) -> Self {
        for &b in bytes {
            self.0 ^= u32::from(b);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
        self
    }

    fn char(self, c: char) -> Self {
        let mut buf = [0u8; 4];
        self.bytes(c.encode_utf8(&mut buf).as_bytes())
    }

    fn bucket(self) -> u32 {
        self.0 % SPARSE_BUCKETS
    }
}

/// Stable hash of a bundle id, used for [`Features::app`].
#[must_use]
pub(crate) fn app_hash(bundle_id: &str) -> u32 {
    Fnv::new().bytes(bundle_id.as_bytes()).0
}

fn lang_sign(l: Lang) -> f32 {
    match l {
        Lang::Vi => 1.0,
        Lang::En => -1.0,
    }
}

fn b2f(b: bool) -> f32 {
    if b { 1.0 } else { 0.0 }
}

/// Default extractor (see docs/04-engine-ml.md, "Features"):
/// sparse = char 1..4-grams of `^raw$`, `bundle_id`, `bundle_id x caret cell`,
/// caret cell (8x8 grid or "unknown"); dense = valid_vi, has_telex, length,
/// prev langs (3), current mode, squashed secs_since_switch.
///
/// Sparse ids are FNV-1a 32 hashes (salted per group) modulo
/// [`SPARSE_BUCKETS`]. Order: n-grams (by end position, then length 1..4),
/// then app, app x cell, cell. N-grams are over lowercase chars, including the
/// `^`/`$` boundary unigrams; an empty `raw` (focus prior) yields none.
///
/// Dense layout (exactly [`DENSE_LEN`]):
///
/// | idx | value |
/// | --- | --- |
/// | 0 | `valid_vi` (0/1) |
/// | 1 | `has_telex` (0/1) |
/// | 2 | `min(chars(raw) / 10, 1)` |
/// | 3..6 | `prev_langs[i]`: +1 VI, -1 EN, 0 absent |
/// | 6 | `current`: +1 VI, -1 EN |
/// | 7 | `1 / (1 + secs_since_switch / 10)`; 0 for infinite/NaN |
#[derive(Debug, Clone, Default)]
pub struct HashedExtractor;

impl HashedExtractor {
    fn push_ngrams(raw: &str, out: &mut Vec<u32>) {
        if raw.is_empty() {
            return;
        }
        let body = raw.chars().flat_map(char::to_lowercase).take(MAX_RAW_CHARS);
        let seq = core::iter::once('^')
            .chain(body)
            .chain(core::iter::once('$'));
        // Sliding window of the last MAX_NGRAM chars; window[0] is the newest.
        let mut window = ['\0'; MAX_NGRAM];
        let mut filled = 0usize;
        for c in seq {
            window.rotate_right(1);
            window[0] = c;
            filled = (filled + 1).min(MAX_NGRAM);
            for n in 1..=filled {
                // Hash oldest-to-newest so "ab" hashes as a then b.
                let h = window[..n]
                    .iter()
                    .rev()
                    .fold(Fnv::salted(salt::NGRAM), |h, &c| h.char(c));
                out.push(h.bucket());
            }
        }
    }
}

impl FeatureExtractor for HashedExtractor {
    fn extract(&self, ctx: &DecisionCtx<'_>, out: &mut Features) {
        out.clear();
        out.app = app_hash(ctx.bundle_id);
        out.current = ctx.current;

        Self::push_ngrams(ctx.raw, &mut out.sparse);
        let cell = ctx.caret.map_or(CELL_UNKNOWN, |c| {
            c.cell(CARET_GRID).min(u32::from(CELL_UNKNOWN)) as u8
        });
        let app = Fnv::salted(salt::APP).bytes(ctx.bundle_id.as_bytes());
        out.sparse.push(app.bucket());
        let app_cell = Fnv::salted(salt::APP_CELL)
            .bytes(ctx.bundle_id.as_bytes())
            .bytes(&[0, cell]);
        out.sparse.push(app_cell.bucket());
        out.sparse
            .push(Fnv::salted(salt::CELL).bytes(&[cell]).bucket());

        let len = ctx.raw.chars().count() as f32;
        let prev = |i: usize| ctx.prev_langs.get(i).map_or(0.0, |&l| lang_sign(l));
        let s = ctx.secs_since_switch;
        let recency = if s.is_finite() {
            1.0 / (1.0 + s.max(0.0) / 10.0)
        } else {
            0.0
        };
        out.dense.extend_from_slice(&[
            b2f(ctx.valid_vi),
            b2f(ctx.has_telex),
            (len / 10.0).min(1.0),
            prev(0),
            prev(1),
            prev(2),
            lang_sign(ctx.current),
            recency,
        ]);
    }
}

#[cfg(test)]
mod tests {
    use nimk_types::CaretRel;

    use super::*;

    fn ctx<'a>(bundle: &'a str, raw: &'a str, prev: &'a [Lang]) -> DecisionCtx<'a> {
        DecisionCtx {
            bundle_id: bundle,
            caret: CaretRel::new(0.0, 0.0),
            raw,
            valid_vi: true,
            has_telex: false,
            prev_langs: prev,
            current: Lang::Vi,
            secs_since_switch: 10.0,
        }
    }

    fn run(c: &DecisionCtx<'_>) -> Features {
        let mut f = Features::default();
        HashedExtractor.extract(c, &mut f);
        f
    }

    #[test]
    fn fnv_known_vector() {
        // Reference FNV-1a 32 values.
        assert_eq!(Fnv::new().0, 0x811c_9dc5);
        assert_eq!(Fnv::new().bytes(b"a").0, 0xe40c_292c);
        assert_eq!(Fnv::new().bytes(b"foobar").0, 0xbf9c_f968);
    }

    #[test]
    fn layout_and_determinism() {
        let f = run(&ctx("com.a", "ab", &[Lang::En]));
        assert_eq!(f, run(&ctx("com.a", "ab", &[Lang::En])));
        // "^ab$" -> 4 chars: 1 + 2 + 3 + 4 n-grams = 10, plus 3 context ids.
        assert_eq!(f.sparse.len(), 13);
        assert!(f.sparse.iter().all(|&id| id < SPARSE_BUCKETS));
        assert_eq!(f.dense, vec![1.0, 0.0, 0.2, -1.0, 0.0, 0.0, 1.0, 0.5]);
        assert_eq!(f.app, app_hash("com.a"));
        assert_eq!(f.current, Lang::Vi);
    }

    #[test]
    fn empty_raw_has_only_context_ids() {
        let mut c = ctx("com.a", "", &[]);
        c.caret = None;
        c.secs_since_switch = f32::INFINITY;
        let f = run(&c);
        assert_eq!(f.sparse.len(), 3);
        assert_eq!(f.dense.len(), DENSE_LEN);
        assert_eq!(f.dense[7], 0.0);
    }

    #[test]
    fn case_insensitive_and_app_sensitive() {
        let a = run(&ctx("com.a", "HaS", &[]));
        let b = run(&ctx("com.a", "has", &[]));
        assert_eq!(a.sparse, b.sparse);
        let c = run(&ctx("com.b", "has", &[]));
        assert_eq!(a.sparse[..14], c.sparse[..14]);
        assert_ne!(a.sparse[14..], c.sparse[14..]);
    }

    #[test]
    fn groups_use_distinct_salts() {
        let ngram = Fnv::salted(salt::NGRAM).char('a').bucket();
        let app = Fnv::salted(salt::APP).bytes(b"a").bucket();
        assert_ne!(ngram, app);
    }

    #[test]
    fn long_raw_is_capped() {
        let raw = "a".repeat(100);
        let f = run(&ctx("x", &raw, &[]));
        assert_eq!(f.sparse.len(), 4 * (MAX_RAW_CHARS + 2) - 6 + 3);
        assert_eq!(f.dense[2], 1.0);
    }
}
