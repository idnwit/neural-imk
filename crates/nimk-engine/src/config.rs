use nimk_policy::ModelKind;

/// Where training runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Threading {
    /// Trainer and log writer on background threads (production).
    Background,
    /// Everything on the caller's thread, deterministic (tests, replay).
    Inline,
}

/// Engine tuning knobs. Defaults match docs/04-engine-ml.md. Serialized
/// into the `SessionStart` event so replay can rebuild the same engine.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct EngineConfig {
    /// Action model to build when no snapshot exists. Default `LinearAction`:
    /// same accuracy as `GruAction` on synthetic data at ~300x lower latency.
    pub policy: ModelKind,
    /// Seed for model initialization.
    pub seed: u64,
    /// Hysteresis: switch away from the current mode only if `|p - 0.5| > margin`.
    pub switch_margin: f32,
    /// Detect the switch hotkey from key events. Replay sets `false` and
    /// feeds logged labels through `Engine::inject_label` instead.
    pub hotkeys: bool,
    /// Max seconds between a Shift down and up to count as a tap.
    pub shift_tap_secs: f64,
    /// Accept a double Shift tap as the switch hotkey (ADR 0008;
    /// independent of `ctrl_space`; `hotkeys = false` disables both).
    pub double_shift: bool,
    /// Max seconds between the releases of the two taps of a double Shift.
    pub double_shift_secs: f64,
    /// Also accept Ctrl+Space as the switch hotkey.
    pub ctrl_space: bool,
    /// Words after a commit during which a hotkey still corrects that commit.
    pub correction_words: usize,
    /// Seconds after a commit before its implicit label is finalized.
    pub label_timeout_secs: f64,
    /// Weight of implicit (uncorrected) labels.
    pub implicit_weight: f32,
    /// Weight of the edit-head loss in online examples (ADR 0007: 0 = the
    /// edit head is only trained by pre-training / `Engine::teach`).
    pub edit_online_weight: f32,
    /// Teacher-forced examples (`Engine::teach`) per learner update; 1 =
    /// one update per key. Larger batches make pre-training faster and
    /// smoother (`nimk-synth` uses 128).
    pub teach_batch: usize,
    /// Samples re-trained around a correction.
    pub correction_window: usize,
    /// Gradient steps over the correction window.
    pub correction_steps: usize,
    /// Replay buffer capacity.
    pub replay_capacity: usize,
    /// Train one random minibatch every N new samples.
    pub replay_every: usize,
    /// Minibatch size for replay training.
    pub replay_batch: usize,
    /// Seconds between model snapshots to storage.
    pub snapshot_every_secs: f64,
    /// Background or inline training.
    pub threading: Threading,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            policy: ModelKind::LinearAction,
            seed: 0x5eed,
            switch_margin: 0.2,
            hotkeys: true,
            shift_tap_secs: 0.3,
            double_shift: true,
            double_shift_secs: 0.4,
            ctrl_space: true,
            correction_words: 2,
            label_timeout_secs: 10.0,
            implicit_weight: 0.3,
            edit_online_weight: 0.0,
            teach_batch: 1,
            correction_window: 20,
            correction_steps: 5,
            replay_capacity: 10_000,
            replay_every: 50,
            replay_batch: 32,
            snapshot_every_secs: 300.0,
            threading: Threading::Background,
        }
    }
}
