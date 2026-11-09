//! A pipeline stage that may change the buffer or the output: a pure
//! function of its input (state lives in the engine and is passed in).
//! Observers that must not change anything are hooks (`nimk-hook`).

/// One pure pipeline stage.
pub trait Transform {
    /// Data the stage reads.
    type Input: ?Sized;
    /// Data the stage produces.
    type Output;
    /// Runs the stage. Must not panic and must be deterministic.
    fn apply(&self, input: &Self::Input) -> Self::Output;
}
