//! The `model_stats` hook: per-model quality counters from `model_score`
//! records (docs/05-storage-and-log.md).

mod tracker;

use std::io;

use nimk_types::{Event, Record};

use tracker::{Scored, StatsTracker};

use crate::{Hook, HookCx};

/// Model kind names, in tracker order (`ModelKind::index`).
const KINDS: [&str; 2] = ["linear_action", "gru_action"];

/// Tracker slot of a model kind name.
fn slot(kind: &str) -> Option<usize> {
    KINDS.iter().position(|k| *k == kind)
}

/// Counts `model_score` records per model kind; the summary is
/// `{"linear_action": ModelStats, "gru_action": ModelStats}` and each
/// kind's counters persist at `hooks/model_stats/<kind>`.
#[derive(Default)]
pub struct ModelStatsHook {
    trackers: [StatsTracker; 2],
}

impl ModelStatsHook {
    /// A hook with empty counters (`init` loads the saved ones).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn publish(&self, cx: &mut HookCx<'_>) {
        let [linear, gru] = &self.trackers;
        let json = serde_json::json!({
            KINDS[0]: linear.summary(),
            KINDS[1]: gru.summary(),
        });
        cx.summary(json.to_string());
    }

    /// Writes `tracker` to `hooks/model_stats/<kind>`.
    fn save(kind: &str, tracker: &StatsTracker, cx: &HookCx<'_>) -> io::Result<()> {
        let bytes = tracker.to_bytes().map_err(io::Error::other)?;
        cx.put(kind, &bytes)
    }
}

/// Restores the counters of `kind`: the hook's own blob, else the legacy
/// `stats/<kind>` (adopted and written to the new key once). Unreadable or
/// unusable blobs give empty counters and a diagnostic.
fn load(kind: &str, cx: &mut HookCx<'_>) -> StatsTracker {
    let own = format!("hooks/model_stats/{kind}");
    let (key, bytes, legacy) = match cx.get(kind) {
        Ok(Some(bytes)) => (own, bytes, false),
        Ok(None) => {
            let key = format!("stats/{kind}");
            match cx.get_legacy(&key) {
                Ok(Some(bytes)) => (key, bytes, true),
                Ok(None) => return StatsTracker::default(),
                Err(e) => {
                    cx.diagnostic(format!("cannot read {key} ({e}); counting from zero"));
                    return StatsTracker::default();
                }
            }
        }
        Err(e) => {
            cx.diagnostic(format!("cannot read {own} ({e}); counting from zero"));
            return StatsTracker::default();
        }
    };
    let tracker = match StatsTracker::from_bytes(&bytes) {
        Ok(tracker) => tracker,
        Err(e) => {
            cx.diagnostic(format!("{key} unusable ({e}); counting from zero"));
            return StatsTracker::default();
        }
    };
    if legacy && let Err(e) = cx.put(kind, &bytes) {
        cx.diagnostic(format!("cannot migrate {key} ({e})"));
    }
    tracker
}

impl Hook for ModelStatsHook {
    fn name(&self) -> &'static str {
        "model_stats"
    }

    fn init(&mut self, cx: &mut HookCx<'_>) {
        for (kind, tracker) in KINDS.iter().zip(self.trackers.iter_mut()) {
            *tracker = load(kind, cx);
        }
        self.publish(cx);
    }

    fn on_record(&mut self, rec: &Record, cx: &mut HookCx<'_>) {
        match &rec.event {
            Event::ModelScore {
                kind,
                word_id,
                category,
                correct,
                display_hits,
                display_total,
                flips,
                amend,
                served,
            } => {
                let Some(tracker) = slot(kind).and_then(|i| self.trackers.get_mut(i)) else {
                    return cx.diagnostic(format!("model_score for unknown model {kind}; ignored"));
                };
                if *amend {
                    tracker.amend(*word_id, *correct, *display_hits);
                } else {
                    tracker.record(Scored {
                        word_id: *word_id,
                        category: *category,
                        served: *served,
                        correct: *correct,
                        keys: *display_total,
                        matched: *display_hits,
                        flips: *flips,
                    });
                }
            }
            Event::ModelReset { kind } => {
                let Some(tracker) = slot(kind).and_then(|i| self.trackers.get_mut(i)) else {
                    return cx.diagnostic(format!("model_reset for unknown model {kind}; ignored"));
                };
                *tracker = StatsTracker::default();
                // Saved at once, so a restart never brings the old counters back.
                match Self::save(kind, tracker, cx) {
                    Ok(()) => tracker.mark_saved(),
                    Err(e) => cx.diagnostic(format!("cannot save reset counters of {kind} ({e})")),
                }
            }
            _ => return,
        }
        self.publish(cx);
    }

    fn flush(&mut self, cx: &mut HookCx<'_>) -> io::Result<()> {
        let mut result = Ok(());
        for (kind, tracker) in KINDS.iter().zip(self.trackers.iter_mut()) {
            if !tracker.is_dirty() {
                continue;
            }
            match Self::save(kind, tracker, cx) {
                Ok(()) => tracker.mark_saved(),
                Err(e) => {
                    if result.is_ok() {
                        result = Err(e);
                    }
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::{Hook, HookCx, HookMsg};
    use nimk_storage::{MemStorage, Storage};
    use nimk_types::{Event, ModelStats, Record, StatCategory};

    fn score(word_id: u64, correct: bool, amend: bool) -> Record {
        Record {
            v: 1,
            seq: word_id,
            t: 1.0,
            session: "s".into(),
            event: Event::ModelScore {
                kind: "gru_action".into(),
                word_id,
                category: StatCategory::Other,
                correct,
                display_hits: 2,
                display_total: 3,
                flips: 1,
                amend,
                served: true,
            },
        }
    }

    fn summary(out: &[HookMsg]) -> serde_json::Value {
        match out
            .iter()
            .rev()
            .find(|m| matches!(m, HookMsg::Summary { .. }))
        {
            Some(HookMsg::Summary { json, .. }) => serde_json::from_str(json).unwrap(),
            _ => panic!("no summary"),
        }
    }

    #[test]
    fn counts_and_amends() {
        let s = MemStorage::new();
        let mut out = Vec::new();
        let mut h = ModelStatsHook::new();
        h.init(&mut HookCx::new("model_stats", s.blobs(), &mut out));
        for r in [
            score(0, true, false),
            score(1, false, false),
            score(1, true, true),
        ] {
            h.on_record(&r, &mut HookCx::new("model_stats", s.blobs(), &mut out));
        }
        let gru: ModelStats = serde_json::from_value(summary(&out)["gru_action"].clone()).unwrap();
        assert_eq!((gru.words_total, gru.corrected_total), (2, 0));
        h.flush(&mut HookCx::new("model_stats", s.blobs(), &mut out))
            .unwrap();
        assert!(
            s.blobs()
                .get("hooks/model_stats/gru_action")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn legacy_stats_blob_is_migrated_once() {
        let s = MemStorage::new();
        let mut legacy = StatsTracker::default();
        legacy.record(Scored {
            word_id: 0,
            category: StatCategory::Other,
            served: true,
            correct: false,
            keys: 3,
            matched: 1,
            flips: 0,
        });
        s.blobs()
            .put("stats/gru_action", &legacy.to_bytes().unwrap())
            .unwrap();
        let mut out = Vec::new();
        let mut h = ModelStatsHook::new();
        h.init(&mut HookCx::new("model_stats", s.blobs(), &mut out));
        let gru: ModelStats = serde_json::from_value(summary(&out)["gru_action"].clone()).unwrap();
        assert_eq!((gru.words_total, gru.corrected_total), (1, 1));
        assert!(
            s.blobs()
                .get("hooks/model_stats/gru_action")
                .unwrap()
                .is_some(),
            "written at init"
        );
        // Second start reads the new key, not the legacy one.
        s.blobs().put("stats/gru_action", b"garbage").unwrap();
        let mut out2 = Vec::new();
        ModelStatsHook::new().init(&mut HookCx::new("model_stats", s.blobs(), &mut out2));
        assert!(!out2.iter().any(|m| matches!(m, HookMsg::Diagnostic(_))));
    }

    #[test]
    fn reset_zeroes_one_kind_and_saves_at_once() {
        let s = MemStorage::new();
        let mut out = Vec::new();
        let mut h = ModelStatsHook::new();
        h.init(&mut HookCx::new("model_stats", s.blobs(), &mut out));
        h.on_record(
            &score(0, false, false),
            &mut HookCx::new("model_stats", s.blobs(), &mut out),
        );
        let reset = Record {
            v: 1,
            seq: 9,
            t: 2.0,
            session: "s".into(),
            event: Event::ModelReset {
                kind: "gru_action".into(),
            },
        };
        h.on_record(&reset, &mut HookCx::new("model_stats", s.blobs(), &mut out));
        let gru: ModelStats = serde_json::from_value(summary(&out)["gru_action"].clone()).unwrap();
        assert_eq!(gru, ModelStats::default());
        let saved = s
            .blobs()
            .get("hooks/model_stats/gru_action")
            .unwrap()
            .unwrap();
        assert_eq!(
            StatsTracker::from_bytes(&saved).unwrap().summary(),
            ModelStats::default()
        );
    }
}
