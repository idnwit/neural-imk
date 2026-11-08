use crate::Lang;

/// Model input produced by a feature extractor.
///
/// `app` and `current` are named slots so non-neural policies (e.g. "last
/// mode per app") can read them without decoding `sparse`; neural models
/// receive the same information through `sparse`/`dense`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Features {
    /// Stable hash of the app's bundle id.
    pub app: u32,
    /// Mode in effect when the decision is made.
    pub current: Lang,
    /// Hashed sparse feature ids; the extractor defines the bucket count.
    pub sparse: Vec<u32>,
    /// Dense features; the extractor defines layout and length.
    pub dense: Vec<f32>,
}

impl Features {
    /// Clears all vectors while keeping their capacity, for reuse on the hot path.
    pub fn clear(&mut self) {
        self.app = 0;
        self.current = Lang::En;
        self.sparse.clear();
        self.dense.clear();
    }
}
