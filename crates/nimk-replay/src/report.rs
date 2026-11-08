//! Report types (serialized to `report.json`) and the stdout table.

use std::collections::BTreeMap;

use nimk_policy::PolicyKind;
use serde::{Deserialize, Serialize};

use crate::Counts;

/// CLI / report name of a policy kind.
#[must_use]
pub const fn policy_name(kind: PolicyKind) -> &'static str {
    match kind {
        PolicyKind::Baseline => "baseline",
        PolicyKind::Linear => "linear",
        PolicyKind::TinyMlp => "tiny_mlp",
    }
}

/// Counters plus derived ratios; a ratio is `None` when its denominator is 0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    /// Raw counters.
    pub counts: Counts,
    /// `correct_words / labelled_words`.
    pub word_accuracy: Option<f64>,
    /// `ambiguous_correct / ambiguous_words`.
    pub ambiguous_accuracy: Option<f64>,
    /// `auto_switches_agreed / auto_switches_labelled`.
    pub auto_switch_precision: Option<f64>,
    /// `(labelled_words - correct_words) * 1000 / words`: each wrong word
    /// would have needed one manual switch.
    pub manual_switches_per_1000: Option<f64>,
    /// `priors_correct / priors_labelled`.
    pub prior_accuracy: Option<f64>,
}

fn ratio(num: u64, den: u64) -> Option<f64> {
    (den > 0).then(|| num as f64 / den as f64)
}

impl Metrics {
    /// Derives the ratios from `c`.
    #[must_use]
    pub fn from_counts(c: &Counts) -> Self {
        let wrong = c.labelled_words.saturating_sub(c.correct_words);
        Self {
            counts: *c,
            word_accuracy: ratio(c.correct_words, c.labelled_words),
            ambiguous_accuracy: ratio(c.ambiguous_correct, c.ambiguous_words),
            auto_switch_precision: ratio(c.auto_switches_agreed, c.auto_switches_labelled),
            manual_switches_per_1000: ratio(wrong * 1000, c.words),
            prior_accuracy: ratio(c.priors_correct, c.priors_labelled),
        }
    }
}

/// Results for one policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyReport {
    /// Policy name (`baseline`, `linear`, `tiny_mlp`).
    pub policy: String,
    /// Policy id serving at the end of the last session, e.g. `linear@v812`.
    pub final_policy_id: Option<String>,
    /// All bundles together.
    pub overall: Metrics,
    /// Per app bundle id.
    pub by_bundle: BTreeMap<String, Metrics>,
}

/// Run metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunMeta {
    /// Policies requested, in order.
    pub policies: Vec<String>,
    /// Segment prefix filter.
    pub from: Option<String>,
    /// Seed override.
    pub seed: Option<u64>,
    /// Segments read.
    pub segments: usize,
    /// Sessions replayed.
    pub sessions: usize,
    /// Records parsed.
    pub records: usize,
    /// Malformed lines skipped.
    pub malformed: usize,
    /// Total warnings (skipped lines, config fallbacks, engine diagnostics).
    pub warning_count: usize,
    /// First warnings, capped.
    pub warnings: Vec<String>,
}

/// Full replay report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// Run metadata.
    pub meta: RunMeta,
    /// One entry per policy; empty when there were no sessions.
    pub policies: Vec<PolicyReport>,
}

fn pct(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_owned(), |v| format!("{:.1}%", v * 100.0))
}

fn num(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_owned(), |v| format!("{v:.1}"))
}

const HEADERS: [&str; 10] = [
    "policy",
    "scope",
    "words",
    "labelled",
    "acc",
    "amb_acc",
    "auto_sw",
    "auto_prec",
    "manual/1k",
    "prior_acc",
];

fn row(policy: &str, scope: &str, m: &Metrics) -> [String; 10] {
    let c = &m.counts;
    [
        policy.to_owned(),
        scope.to_owned(),
        c.words.to_string(),
        c.labelled_words.to_string(),
        pct(m.word_accuracy),
        pct(m.ambiguous_accuracy),
        c.auto_switches.to_string(),
        pct(m.auto_switch_precision),
        num(m.manual_switches_per_1000),
        pct(m.prior_accuracy),
    ]
}

/// Renders the report as an aligned text table (one overall row per policy,
/// then one row per bundle id).
#[must_use]
pub fn render_table(report: &Report) -> String {
    let meta = &report.meta;
    let mut out = format!(
        "segments {}  sessions {}  records {}  malformed {}  warnings {}\n",
        meta.segments, meta.sessions, meta.records, meta.malformed, meta.warning_count
    );
    if report.policies.is_empty() {
        out.push_str("no sessions found\n");
        return out;
    }
    let mut rows = vec![HEADERS.map(str::to_owned)];
    for p in &report.policies {
        rows.push(row(&p.policy, "ALL", &p.overall));
        for (bundle, m) in &p.by_bundle {
            rows.push(row("", bundle, m));
        }
    }
    let mut widths = [0usize; 10];
    for r in &rows {
        for (w, cell) in widths.iter_mut().zip(r) {
            *w = (*w).max(cell.chars().count());
        }
    }
    for r in &rows {
        let mut line = String::new();
        for (i, (cell, w)) in r.iter().zip(widths).enumerate() {
            // Text columns left-aligned, numbers right-aligned.
            let padded = if i < 2 {
                format!("{cell:<w$}  ")
            } else {
                format!("{cell:>w$}  ")
            };
            line.push_str(&padded);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratios_handle_zero_denominators() {
        let m = Metrics::from_counts(&Counts::default());
        assert_eq!(m.word_accuracy, None);
        assert_eq!(m.manual_switches_per_1000, None);
        let c = Counts {
            words: 4,
            labelled_words: 4,
            correct_words: 3,
            ..Counts::default()
        };
        let m = Metrics::from_counts(&c);
        assert_eq!(m.word_accuracy, Some(0.75));
        assert_eq!(m.manual_switches_per_1000, Some(250.0));
    }
}
