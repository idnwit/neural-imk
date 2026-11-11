//! Engine configuration. Every default lives in `config/default.toml`
//! (embedded with `include_str!`); this file holds the types, the layered
//! TOML merge and the validation. The engine never reads files: the host
//! passes the text of the user's override (docs/11-config.md).

use nimk_policy::ModelKind;
use serde::{Deserialize, Serialize};

/// The embedded defaults: the single source of truth.
pub const DEFAULT_TOML: &str = include_str!("../config/default.toml");

/// Where training runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Threading {
    /// Trainer and hooks (event log included) on background threads (production).
    Background,
    /// Everything on the caller's thread, deterministic (tests, replay).
    Inline,
}

/// Options of one model (`[models.<kind>]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelOptions {
    /// Train this model online from labels, also while it is not the active
    /// one (shadow training, ADR 0013). Off freezes it: no updates, no
    /// saves, no shadow statistics.
    pub online_training: bool,
}

impl Default for ModelOptions {
    fn default() -> Self {
        Self {
            online_training: true,
        }
    }
}

/// Per-model options, one table per [`ModelKind`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelsConfig {
    /// Options of `gru_action`.
    pub gru_action: ModelOptions,
    /// Options of `linear_action`.
    pub linear_action: ModelOptions,
}

impl ModelsConfig {
    /// Whether online training of `kind` is on.
    #[must_use]
    pub fn online_training(&self, kind: ModelKind) -> bool {
        match kind {
            ModelKind::GruAction => self.gru_action.online_training,
            ModelKind::LinearAction => self.linear_action.online_training,
        }
    }

    /// Turns online training of `kind` on or off.
    pub fn set_online_training(&mut self, kind: ModelKind, on: bool) {
        match kind {
            ModelKind::GruAction => self.gru_action.online_training = on,
            ModelKind::LinearAction => self.linear_action.online_training = on,
        }
    }
}

/// Parameters a category may override (the keys of `[app_category.params]`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CategoryParams {
    /// See [`EngineConfig::switch_margin`].
    pub switch_margin: Option<f32>,
    /// See [`EngineConfig::idle_commit_secs`].
    pub idle_commit_secs: Option<f64>,
    /// See [`EngineConfig::double_shift`].
    pub double_shift: Option<bool>,
    /// See [`EngineConfig::double_shift_secs`].
    pub double_shift_secs: Option<f64>,
    /// See [`EngineConfig::shift_tap_secs`].
    pub shift_tap_secs: Option<f64>,
    /// See [`EngineConfig::ctrl_space`].
    pub ctrl_space: Option<bool>,
    /// See [`EngineConfig::extra_letters`].
    pub extra_letters: Option<String>,
    /// See [`EngineConfig::extra_boundaries`].
    pub extra_boundaries: Option<String>,
    /// See [`EngineConfig::commit_keycodes`].
    pub commit_keycodes: Option<Vec<u16>>,
    /// See [`EngineConfig::command_triggers`].
    pub command_triggers: Option<Vec<String>>,
    /// See [`EngineConfig::command_end_chars`].
    pub command_end_chars: Option<String>,
    /// See [`EngineConfig::command_end_keycodes`].
    pub command_end_keycodes: Option<Vec<u16>>,
    /// See [`EngineConfig::retro_flip`].
    pub retro_flip: Option<bool>,
    /// See [`EngineConfig::retro_flip_terminal`].
    pub retro_flip_terminal: Option<bool>,
}

/// A group of apps sharing parameter overrides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppCategory {
    /// Category name (`terminal`, `ide`, ...); the merge key across layers.
    pub name: String,
    /// Bundle ids; an entry ending in `*` matches by prefix.
    pub bundle_ids: Vec<String>,
    /// Overrides while an app of this category has focus.
    #[serde(default)]
    pub params: CategoryParams,
}

impl AppCategory {
    fn matches(&self, bundle_id: &str) -> bool {
        self.bundle_ids.iter().any(|b| match b.strip_suffix('*') {
            Some(prefix) => bundle_id.starts_with(prefix),
            None => b == bundle_id,
        })
    }
}

/// A rejected configuration value (the default stays in force).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Key path, e.g. `switch_margin` or `app_category.ide.params.idle_commit_secs`.
    pub key: String,
    /// Why the value was rejected.
    pub message: String,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "config: {}: {}", self.key, self.message)
    }
}

/// Engine tuning knobs; defaults come from `config/default.toml`.
/// Serialized into the `SessionStart` event so replay can rebuild the same
/// engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineConfig {
    /// Action model to build when no snapshot exists.
    pub policy: ModelKind,
    /// Seed for model initialization.
    pub seed: u64,
    /// Hysteresis: switch away from the current rendering only if
    /// `|p - 0.5| > margin` (mid-word display only; the commit uses the
    /// fresh decision).
    pub switch_margin: f32,
    /// Detect the switch hotkey from key events. Replay sets `false` and
    /// feeds logged labels through `Engine::inject_label` instead.
    pub hotkeys: bool,
    /// Weight of implicit (uncorrected) labels.
    pub implicit_weight: f32,
    /// Weight of the edit-head loss in online examples (ADR 0007).
    pub edit_online_weight: f32,
    /// Words after a commit during which a hotkey still corrects that commit.
    pub correction_words: usize,
    /// Seconds after a commit before its implicit label is finalized.
    pub label_timeout_secs: f64,
    /// Teacher-forced examples (`Engine::teach`) per learner update.
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
    /// Accept a double Shift tap as the switch hotkey (`hotkeys = false`
    /// disables both hotkeys).
    pub double_shift: bool,
    /// Max seconds between the releases of the two taps of a double Shift.
    pub double_shift_secs: f64,
    /// Max seconds between a Shift down and up to count as a tap.
    pub shift_tap_secs: f64,
    /// Also accept Ctrl+Space as the switch hotkey.
    pub ctrl_space: bool,
    /// Seconds without a key after which the host commits the composing or
    /// pending word (ADR 0012). The engine has no clock: the host reads
    /// [`crate::Engine::idle_commit_secs`] and calls `Engine::commit_pending`.
    /// 0 = never.
    pub idle_commit_secs: f64,
    /// Characters that count as language although they are not alphabetic.
    pub extra_letters: String,
    /// Characters that count as boundaries although they are alphabetic.
    pub extra_boundaries: String,
    /// Keycodes that commit the word and then reach the app.
    pub commit_keycodes: Vec<u16>,
    /// Characters that, typed with nothing composing, start raw mode: each
    /// is one character, not a letter, digit or whitespace.
    pub command_triggers: Vec<String>,
    /// Characters that end raw mode (and still reach the app).
    pub command_end_chars: String,
    /// Keycodes that end raw mode (Return, Tab, Escape, navigation).
    pub command_end_keycodes: Vec<u16>,
    /// Caret flip in clients that expose their text (ADR 0015): the hotkey
    /// with no pending word switches the word at the caret in place.
    pub retro_flip: bool,
    /// Caret flip in clients that expose no text (terminals), through the
    /// host's line tracker and control bytes (ADR 0015).
    pub retro_flip_terminal: bool,
    /// Per-model options. The IME menu's saved choices (blob
    /// `settings/models`) override these at start.
    #[serde(default)]
    pub models: ModelsConfig,
    /// Per-app-category overrides; the first match wins.
    pub app_category: Vec<AppCategory>,
}

/// The parameters in force for the focused app: the base values with the
/// matching category's overrides applied.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LiveParams {
    pub(crate) switch_margin: f32,
    pub(crate) idle_commit_secs: f64,
    pub(crate) double_shift: bool,
    pub(crate) double_shift_secs: f64,
    pub(crate) shift_tap_secs: f64,
    pub(crate) ctrl_space: bool,
    pub(crate) extra_letters: String,
    pub(crate) extra_boundaries: String,
    pub(crate) commit_keycodes: Vec<u16>,
    pub(crate) command_triggers: Vec<String>,
    pub(crate) command_end_chars: String,
    pub(crate) command_end_keycodes: Vec<u16>,
    pub(crate) retro_flip: bool,
    pub(crate) retro_flip_terminal: bool,
}

impl LiveParams {
    /// Whether `c` is a language character (a letter of a word).
    pub(crate) fn is_letter(&self, c: char) -> bool {
        (c.is_alphabetic() || self.extra_letters.contains(c)) && !self.extra_boundaries.contains(c)
    }

    /// Whether `c` starts raw mode.
    pub(crate) fn is_trigger(&self, c: char) -> bool {
        self.command_triggers
            .iter()
            .any(|t| t.chars().eq(std::iter::once(c)))
    }
}

/// One violated range.
struct Violation {
    category: Option<String>,
    key: &'static str,
    message: String,
}

fn in_range(v: f64, lo: f64, hi: f64) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}

impl EngineConfig {
    /// The parameters for the app with `bundle_id` (`""` = no app).
    pub(crate) fn live_for(&self, bundle_id: &str) -> LiveParams {
        let mut live = self.base_live();
        if let Some(cat) = self.app_category.iter().find(|c| c.matches(bundle_id)) {
            apply_params(&mut live, &cat.params);
        }
        live
    }

    /// Name of the category of the app with `bundle_id`, if any matches.
    pub(crate) fn category_name(&self, bundle_id: &str) -> Option<&str> {
        self.app_category
            .iter()
            .find(|c| c.matches(bundle_id))
            .map(|c| c.name.as_str())
    }

    fn base_live(&self) -> LiveParams {
        LiveParams {
            switch_margin: self.switch_margin,
            idle_commit_secs: self.idle_commit_secs,
            double_shift: self.double_shift,
            double_shift_secs: self.double_shift_secs,
            shift_tap_secs: self.shift_tap_secs,
            ctrl_space: self.ctrl_space,
            extra_letters: self.extra_letters.clone(),
            extra_boundaries: self.extra_boundaries.clone(),
            commit_keycodes: self.commit_keycodes.clone(),
            command_triggers: self.command_triggers.clone(),
            command_end_chars: self.command_end_chars.clone(),
            command_end_keycodes: self.command_end_keycodes.clone(),
            retro_flip: self.retro_flip,
            retro_flip_terminal: self.retro_flip_terminal,
        }
    }

    /// Explicit range and shape checks of every value.
    fn violations(&self) -> Vec<Violation> {
        let mut v = Vec::new();
        let mut bad = |category: Option<&str>, key: &'static str, message: &str| {
            v.push(Violation {
                category: category.map(str::to_owned),
                key,
                message: message.to_owned(),
            });
        };
        let top = |key: &'static str,
                   ok: bool,
                   msg: &str,
                   bad: &mut dyn FnMut(Option<&str>, &'static str, &str)| {
            if !ok {
                bad(None, key, msg);
            }
        };
        top(
            "implicit_weight",
            in_range(f64::from(self.implicit_weight), 0.0, 1.0),
            "must be in [0, 1]",
            &mut bad,
        );
        top(
            "edit_online_weight",
            in_range(f64::from(self.edit_online_weight), 0.0, 1.0),
            "must be in [0, 1]",
            &mut bad,
        );
        top(
            "correction_words",
            self.correction_words <= 20,
            "must be in [0, 20]",
            &mut bad,
        );
        top(
            "label_timeout_secs",
            in_range(self.label_timeout_secs, 0.5, 600.0),
            "must be in [0.5, 600]",
            &mut bad,
        );
        top(
            "teach_batch",
            (1..=4096).contains(&self.teach_batch),
            "must be in [1, 4096]",
            &mut bad,
        );
        top(
            "correction_window",
            (1..=1000).contains(&self.correction_window),
            "must be in [1, 1000]",
            &mut bad,
        );
        top(
            "correction_steps",
            self.correction_steps <= 100,
            "must be in [0, 100]",
            &mut bad,
        );
        top(
            "replay_capacity",
            (16..=1_000_000).contains(&self.replay_capacity),
            "must be in [16, 1000000]",
            &mut bad,
        );
        top(
            "replay_every",
            self.replay_every <= 100_000,
            "must be in [0, 100000]",
            &mut bad,
        );
        top(
            "replay_batch",
            (1..=4096).contains(&self.replay_batch),
            "must be in [1, 4096]",
            &mut bad,
        );
        top(
            "snapshot_every_secs",
            in_range(self.snapshot_every_secs, 10.0, 86_400.0),
            "must be in [10, 86400]",
            &mut bad,
        );
        self.live_violations(None, &self.base_live(), &mut v);
        for cat in &self.app_category {
            if cat.name.trim().is_empty() {
                v.push(Violation {
                    category: Some(cat.name.clone()),
                    key: "name",
                    message: "must not be empty".into(),
                });
            }
            let mut live = self.base_live();
            apply_params(&mut live, &cat.params);
            let mut cv = Vec::new();
            self.live_violations(Some(&cat.name), &live, &mut cv);
            // Only the keys the category actually overrides can be its fault.
            cv.retain(|x| params_sets(&cat.params, x.key));
            v.extend(cv);
        }
        v
    }

    fn live_violations(&self, cat: Option<&str>, l: &LiveParams, v: &mut Vec<Violation>) {
        let mut bad = |key: &'static str, message: &str| {
            v.push(Violation {
                category: cat.map(str::to_owned),
                key,
                message: message.to_owned(),
            });
        };
        if !in_range(f64::from(l.switch_margin), 0.0, 0.5) {
            bad("switch_margin", "must be in [0, 0.5]");
        }
        if !(l.idle_commit_secs == 0.0 || in_range(l.idle_commit_secs, 0.3, 10.0)) {
            bad("idle_commit_secs", "must be 0 (off) or in [0.3, 10]");
        }
        if !in_range(l.double_shift_secs, 0.05, 2.0) {
            bad("double_shift_secs", "must be in [0.05, 2]");
        }
        if !in_range(l.shift_tap_secs, 0.05, 1.0) {
            bad("shift_tap_secs", "must be in [0.05, 1]");
        }
        if l.extra_letters
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
        {
            bad(
                "extra_letters",
                "must not contain control or whitespace characters",
            );
        }
        if l.extra_boundaries.chars().any(char::is_control) {
            bad("extra_boundaries", "must not contain control characters");
        }
        for (key, codes) in [
            ("commit_keycodes", &l.commit_keycodes),
            ("command_end_keycodes", &l.command_end_keycodes),
        ] {
            if codes.iter().any(|c| *c > 127) {
                bad(key, "keycodes must be in [0, 127]");
            }
        }
        for t in &l.command_triggers {
            let mut chars = t.chars();
            let ok = matches!((chars.next(), chars.next()),
                (Some(c), None) if !c.is_alphanumeric() && !c.is_whitespace() && !c.is_control());
            if !ok {
                bad(
                    "command_triggers",
                    "each trigger must be one character, not a letter, digit or whitespace",
                );
                break;
            }
        }
    }

    /// Builds the configuration: the embedded defaults, then each of
    /// `layers` (TOML text) merged on top field by field. A value that does
    /// not parse as its type or is out of range is dropped with a
    /// [`Diagnostic`]; the previous value stays. Never fails.
    #[must_use]
    pub fn from_toml_layers(layers: &[&str]) -> (Self, Vec<Diagnostic>) {
        let mut diags = Vec::new();
        let Some(mut table) = parse_table(DEFAULT_TOML, "default.toml", &mut diags) else {
            return (Self::inert(), diags);
        };
        for (i, text) in layers.iter().enumerate() {
            if let Some(layer) = parse_table(text, &format!("layer {i}"), &mut diags) {
                apply_layer(&mut table, layer, &mut diags);
            }
        }
        match table.try_into::<Self>() {
            Ok(cfg) => (cfg, diags),
            Err(e) => {
                diags.push(Diagnostic {
                    key: "(all)".into(),
                    message: e.to_string(),
                });
                (Self::inert(), diags)
            }
        }
    }

    /// Last-resort values if the embedded file were broken (never reached:
    /// the unit tests parse it).
    fn inert() -> Self {
        Self {
            policy: ModelKind::LinearAction,
            seed: 0,
            switch_margin: 0.0,
            hotkeys: true,
            implicit_weight: 0.0,
            edit_online_weight: 0.0,
            correction_words: 0,
            label_timeout_secs: 10.0,
            teach_batch: 1,
            correction_window: 1,
            correction_steps: 0,
            replay_capacity: 16,
            replay_every: 0,
            replay_batch: 1,
            snapshot_every_secs: 300.0,
            threading: Threading::Inline,
            double_shift: true,
            double_shift_secs: 0.4,
            shift_tap_secs: 0.3,
            ctrl_space: false,
            idle_commit_secs: 0.0,
            extra_letters: String::new(),
            extra_boundaries: String::new(),
            commit_keycodes: Vec::new(),
            command_triggers: Vec::new(),
            command_end_chars: String::new(),
            command_end_keycodes: Vec::new(),
            retro_flip: false,
            retro_flip_terminal: false,
            models: ModelsConfig::default(),
            app_category: Vec::new(),
        }
    }
}

impl Default for EngineConfig {
    /// The embedded `config/default.toml`.
    fn default() -> Self {
        Self::from_toml_layers(&[]).0
    }
}

fn apply_params(live: &mut LiveParams, p: &CategoryParams) {
    macro_rules! set {
        ($($f:ident),*) => {$( if let Some(v) = &p.$f { live.$f = v.clone(); } )*};
    }
    set!(
        switch_margin,
        idle_commit_secs,
        double_shift,
        double_shift_secs,
        shift_tap_secs,
        ctrl_space,
        extra_letters,
        extra_boundaries,
        commit_keycodes,
        command_triggers,
        command_end_chars,
        command_end_keycodes,
        retro_flip,
        retro_flip_terminal
    );
}

fn params_sets(p: &CategoryParams, key: &str) -> bool {
    match key {
        "switch_margin" => p.switch_margin.is_some(),
        "idle_commit_secs" => p.idle_commit_secs.is_some(),
        "double_shift" => p.double_shift.is_some(),
        "double_shift_secs" => p.double_shift_secs.is_some(),
        "shift_tap_secs" => p.shift_tap_secs.is_some(),
        "ctrl_space" => p.ctrl_space.is_some(),
        "extra_letters" => p.extra_letters.is_some(),
        "extra_boundaries" => p.extra_boundaries.is_some(),
        "commit_keycodes" => p.commit_keycodes.is_some(),
        "command_triggers" => p.command_triggers.is_some(),
        "command_end_chars" => p.command_end_chars.is_some(),
        "command_end_keycodes" => p.command_end_keycodes.is_some(),
        "retro_flip" => p.retro_flip.is_some(),
        "retro_flip_terminal" => p.retro_flip_terminal.is_some(),
        _ => false,
    }
}

fn parse_table(text: &str, what: &str, diags: &mut Vec<Diagnostic>) -> Option<toml::Table> {
    match text.parse::<toml::Table>() {
        Ok(t) => Some(t),
        Err(e) => {
            diags.push(Diagnostic {
                key: what.to_owned(),
                message: format!("not valid TOML, ignored: {}", e.message()),
            });
            None
        }
    }
}

/// Typed check of the working table: `Ok(violations)` or the type error.
fn check(table: &toml::Table) -> Result<Vec<Violation>, String> {
    table
        .clone()
        .try_into::<EngineConfig>()
        .map(|c| c.violations())
        .map_err(|e| e.message().to_owned())
}

fn apply_layer(table: &mut toml::Table, layer: toml::Table, diags: &mut Vec<Diagnostic>) {
    for (key, value) in layer {
        if key == "app_category" {
            apply_categories(table, value, diags);
            continue;
        }
        if key == "models" {
            apply_models(table, value, diags);
            continue;
        }
        if !table.contains_key(&key) {
            diags.push(Diagnostic {
                key,
                message: "unknown key, ignored".into(),
            });
            continue;
        }
        let Some(old) = table.insert(key.clone(), value) else {
            continue;
        };
        let problem = match check(table) {
            Err(e) => Some(e),
            Ok(v) => v
                .into_iter()
                .find(|x| x.category.is_none() && x.key == key)
                .map(|x| x.message),
        };
        if let Some(message) = problem {
            diags.push(Diagnostic {
                key: key.clone(),
                message: format!("{message}; default kept"),
            });
            table.insert(key, old);
        }
    }
}

/// Merges a `[models.<kind>]` layer key by key; a value of the wrong type
/// or an unknown model/key is dropped with a diagnostic.
fn apply_models(table: &mut toml::Table, value: toml::Value, diags: &mut Vec<Diagnostic>) {
    let toml::Value::Table(models) = value else {
        diags.push(Diagnostic {
            key: "models".into(),
            message: "must be a table of tables".into(),
        });
        return;
    };
    for (kind, opts) in models {
        let known = ModelKind::ALL.iter().any(|k| k.name() == kind);
        let toml::Value::Table(opts) = opts else {
            diags.push(Diagnostic {
                key: format!("models.{kind}"),
                message: "must be a table".into(),
            });
            continue;
        };
        if !known {
            diags.push(Diagnostic {
                key: format!("models.{kind}"),
                message: "unknown model, ignored".into(),
            });
            continue;
        }
        for (key, v) in opts {
            let path = format!("models.{kind}.{key}");
            if key != "online_training" {
                diags.push(Diagnostic {
                    key: path,
                    message: "unknown key, ignored".into(),
                });
                continue;
            }
            let Some(toml::Value::Table(cur)) = table.get_mut("models") else {
                continue;
            };
            let slot = cur
                .entry(kind.clone())
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            let toml::Value::Table(slot) = slot else {
                continue;
            };
            let old = slot.insert(key, v);
            if let Err(e) = check(table) {
                diags.push(Diagnostic {
                    key: path,
                    message: format!("{e}; default kept"),
                });
                if let Some(toml::Value::Table(cur)) = table.get_mut("models")
                    && let Some(toml::Value::Table(slot)) = cur.get_mut(&kind)
                {
                    match old {
                        Some(o) => slot.insert("online_training".into(), o),
                        None => slot.remove("online_training"),
                    };
                }
            }
        }
    }
}

fn apply_categories(table: &mut toml::Table, value: toml::Value, diags: &mut Vec<Diagnostic>) {
    let toml::Value::Array(entries) = value else {
        diags.push(Diagnostic {
            key: "app_category".into(),
            message: "must be an array of tables".into(),
        });
        return;
    };
    for entry in entries {
        let toml::Value::Table(entry) = entry else {
            diags.push(Diagnostic {
                key: "app_category".into(),
                message: "entries must be tables".into(),
            });
            continue;
        };
        let Some(name) = entry
            .get("name")
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
        else {
            diags.push(Diagnostic {
                key: "app_category".into(),
                message: "entry without a string `name`, ignored".into(),
            });
            continue;
        };
        let cats = table
            .entry("app_category")
            .or_insert_with(|| toml::Value::Array(Vec::new()));
        let toml::Value::Array(cats) = cats else {
            continue;
        };
        let idx = cats
            .iter()
            .position(|c| c.get("name").and_then(toml::Value::as_str) == Some(&name));
        let backup = idx.and_then(|i| cats.get(i).cloned());
        let merged = merge_category(backup.as_ref(), entry);
        match idx {
            Some(i) => cats[i] = merged,
            None => cats.push(merged),
        }
        settle_category(table, &name, backup, diags);
    }
}

fn merge_category(old: Option<&toml::Value>, new: toml::Table) -> toml::Value {
    let mut out = match old {
        Some(toml::Value::Table(t)) => t.clone(),
        _ => toml::Table::new(),
    };
    for (k, v) in new {
        match (k.as_str(), v) {
            ("params", toml::Value::Table(p)) => {
                let mut cur = match out.remove("params") {
                    Some(toml::Value::Table(t)) => t,
                    _ => toml::Table::new(),
                };
                cur.extend(p);
                out.insert(k, toml::Value::Table(cur));
            }
            (_, v) => {
                out.insert(k, v);
            }
        }
    }
    toml::Value::Table(out)
}

/// Drops, one by one, the params of category `name` that do not parse or
/// are out of range; an entry that cannot be typed at all is reverted.
fn settle_category(
    table: &mut toml::Table,
    name: &str,
    backup: Option<toml::Value>,
    diags: &mut Vec<Diagnostic>,
) {
    for _ in 0..64 {
        match check(table) {
            Ok(v) => {
                let Some(x) = v.into_iter().find(|x| x.category.as_deref() == Some(name)) else {
                    return;
                };
                diags.push(Diagnostic {
                    key: format!("app_category.{name}.params.{}", x.key),
                    message: format!("{}; ignored", x.message),
                });
                if !remove_param(table, name, x.key) {
                    break;
                }
            }
            Err(e) => {
                diags.push(Diagnostic {
                    key: format!("app_category.{name}"),
                    message: format!("{e}; entry ignored"),
                });
                break;
            }
        }
    }
    // Revert to the previous entry, or drop a new one.
    if let Some(toml::Value::Array(cats)) = table.get_mut("app_category") {
        let idx = cats
            .iter()
            .position(|c| c.get("name").and_then(toml::Value::as_str) == Some(name));
        match (idx, backup) {
            (Some(i), Some(b)) => cats[i] = b,
            (Some(i), None) => {
                cats.remove(i);
            }
            _ => {}
        }
    }
}

fn remove_param(table: &mut toml::Table, name: &str, key: &str) -> bool {
    let Some(toml::Value::Array(cats)) = table.get_mut("app_category") else {
        return false;
    };
    for c in cats {
        if c.get("name").and_then(toml::Value::as_str) == Some(name)
            && let Some(toml::Value::Table(p)) = c.get_mut("params")
        {
            return p.remove(key).is_some();
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_default_parses_and_validates() {
        let (cfg, diags) = EngineConfig::from_toml_layers(&[]);
        assert!(diags.is_empty(), "{diags:?}");
        assert_ne!(cfg, EngineConfig::inert());
        assert!(cfg.violations().is_empty());
        assert_eq!(cfg, EngineConfig::default());
        assert_eq!(cfg.policy, ModelKind::GruAction);
    }

    #[test]
    fn every_field_is_in_default_toml() {
        let table: toml::Table = DEFAULT_TOML.parse().unwrap();
        let json = serde_json::to_value(EngineConfig::default()).unwrap();
        for key in json.as_object().unwrap().keys() {
            assert!(table.contains_key(key), "{key} missing from default.toml");
        }
        for key in table.keys() {
            assert!(
                json.get(key).is_some(),
                "{key} is not an EngineConfig field"
            );
        }
    }

    #[test]
    fn category_params_are_documented_keys() {
        // Every `CategoryParams` key is an `EngineConfig` key.
        let all = serde_json::to_value(CategoryParams::default()).unwrap();
        let json = serde_json::to_value(EngineConfig::default()).unwrap();
        for key in all.as_object().unwrap().keys() {
            assert!(json.get(key).is_some(), "{key}");
        }
    }

    #[test]
    fn user_layer_overrides_field_by_field() {
        let (cfg, diags) =
            EngineConfig::from_toml_layers(&["switch_margin = 0.1\nidle_commit_secs = 0\n"]);
        assert!(diags.is_empty(), "{diags:?}");
        assert!((cfg.switch_margin - 0.1).abs() < f32::EPSILON);
        assert_eq!(cfg.idle_commit_secs, 0.0);
        assert!((cfg.double_shift_secs - 0.4).abs() < 1e-9);
        assert_eq!(cfg.app_category.len(), 5);
    }

    #[test]
    fn invalid_values_fall_back_with_a_diagnostic() {
        let user = r#"
switch_margin = 0.9
double_shift_secs = "fast"
idle_commit_secs = 0.1
bogus = 1
extra_letters = "a b"
command_triggers = ["", "ab"]
"#;
        let (cfg, diags) = EngineConfig::from_toml_layers(&[user]);
        let d = EngineConfig::default();
        assert_eq!(cfg, d);
        let keys: Vec<_> = diags.iter().map(|d| d.key.as_str()).collect();
        for k in [
            "switch_margin",
            "double_shift_secs",
            "idle_commit_secs",
            "bogus",
            "extra_letters",
            "command_triggers",
        ] {
            assert!(keys.contains(&k), "{k} not reported: {keys:?}");
        }
    }

    #[test]
    fn models_default_on_and_user_layer_merges_per_model() {
        let d = EngineConfig::default();
        assert!(d.models.online_training(ModelKind::GruAction));
        assert!(d.models.online_training(ModelKind::LinearAction));
        let (cfg, diags) =
            EngineConfig::from_toml_layers(&["[models.gru_action]\nonline_training = false\n"]);
        assert!(diags.is_empty(), "{diags:?}");
        assert!(!cfg.models.online_training(ModelKind::GruAction));
        assert!(cfg.models.online_training(ModelKind::LinearAction));
    }

    #[test]
    fn invalid_model_options_are_reported_and_defaults_kept() {
        let user = r#"
[models.gru_action]
online_training = "yes"
bogus = 1
[models.mystery]
online_training = false
[models.linear_action]
online_training = false
"#;
        let (cfg, diags) = EngineConfig::from_toml_layers(&[user]);
        let keys: Vec<_> = diags.iter().map(|d| d.key.as_str()).collect();
        for k in [
            "models.gru_action.online_training",
            "models.gru_action.bogus",
            "models.mystery",
        ] {
            assert!(keys.contains(&k), "{k} not reported: {keys:?}");
        }
        assert!(cfg.models.online_training(ModelKind::GruAction));
        assert!(
            !cfg.models.online_training(ModelKind::LinearAction),
            "valid sibling kept"
        );
        let (cfg, diags) = EngineConfig::from_toml_layers(&["models = 3"]);
        assert_eq!(diags.len(), 1);
        assert_eq!(cfg, EngineConfig::default());
    }

    #[test]
    fn garbage_text_is_reported_not_fatal() {
        let (cfg, diags) = EngineConfig::from_toml_layers(&["= = ="]);
        assert_eq!(cfg, EngineConfig::default());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn categories_merge_by_name_and_resolve_by_bundle_id() {
        let user = r#"
[[app_category]]
name = "terminal"
[app_category.params]
idle_commit_secs = 3.0
switch_margin = 7.0

[[app_category]]
name = "chat"
bundle_ids = ["com.tinyspeck.slackmacgap"]
[app_category.params]
idle_commit_secs = 0
"#;
        let (cfg, diags) = EngineConfig::from_toml_layers(&[user]);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert!(diags[0].key.ends_with("terminal.params.switch_margin"));
        assert!((cfg.live_for("com.apple.Terminal").idle_commit_secs - 3.0).abs() < 1e-9);
        assert!((cfg.live_for("com.apple.Terminal").switch_margin - 0.02).abs() < 1e-6);
        assert_eq!(
            cfg.live_for("com.tinyspeck.slackmacgap").idle_commit_secs,
            0.0
        );
        assert!((cfg.live_for("com.jetbrains.intellij").idle_commit_secs - 1.0).abs() < 1e-9);
        assert!((cfg.live_for("com.unknown.app").idle_commit_secs - 1.5).abs() < 1e-9);
        assert!((cfg.live_for("").idle_commit_secs - 1.5).abs() < 1e-9);
    }
}
