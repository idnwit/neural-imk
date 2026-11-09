//! The `usage` hook: committed words and corrections per local day, by app
//! and language (docs/05-storage-and-log.md). Counts only; no text is kept.

use std::collections::{BTreeMap, VecDeque};
use std::io;

use nimk_types::{Event, LabelSource, Lang, Record, Target};
use serde::{Deserialize, Serialize};

use crate::{Hook, HookCx};

/// Committed words remembered for attributing later corrections.
const WORD_APPS: usize = 64;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Counts {
    vi: u64,
    en: u64,
    corrected: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Day {
    version: u32,
    date: String,
    apps: BTreeMap<String, Counts>,
    total: Counts,
}

/// Local calendar date (`YYYY-MM-DD`) of `unix_ms` shifted by `offset_min`.
fn local_date(unix_ms: i64, offset_min: i32) -> String {
    let days = (unix_ms.div_euclid(1000) + i64::from(offset_min) * 60).div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Per-day usage counters; the summary is the current day's JSON.
#[derive(Default)]
pub struct UsageHook {
    /// From `session_start` or `clock`: (wall ms, offset min), waiting for
    /// a time base.
    pending: Option<(u64, i32)>,
    /// Clock anchor: (`t` of the first record after `session_start`, wall
    /// ms, offset min). `session_start` itself carries `t = 0`, not the
    /// uptime clock of later records, so it cannot anchor. A `clock` record
    /// likewise re-anchors on the record after it.
    anchor: Option<(f64, u64, i32)>,
    app: String,
    /// Bundle id per committed word id (last [`WORD_APPS`]).
    word_app: VecDeque<(u64, String)>,
    day: Option<Day>,
    dirty: bool,
    warned_no_anchor: bool,
}

impl UsageHook {
    /// A hook with no day loaded (the first commit loads it).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Local date of record time `t`, if the session clock is anchored.
    fn date_at(&self, t: f64) -> Option<String> {
        let (t0, wall, offset) = self.anchor?;
        // Negative deltas clamp to 0; the float-to-int cast saturates.
        #[allow(clippy::cast_possible_truncation)]
        let delta = ((t - t0) * 1000.0).max(0.0) as u64;
        let Ok(ms) = i64::try_from(wall.saturating_add(delta)) else {
            return None;
        };
        Some(local_date(ms, offset))
    }

    /// Writes the current day if it changed.
    fn save(&mut self, cx: &HookCx<'_>) -> io::Result<()> {
        let Some(day) = self.day.as_ref().filter(|_| self.dirty) else {
            return Ok(());
        };
        let bytes = serde_json::to_vec(day).map_err(io::Error::other)?;
        cx.put(&day.date, &bytes)?;
        self.dirty = false;
        Ok(())
    }

    /// Makes `date` the current day, saving the previous one first.
    fn switch_to(&mut self, date: &str, cx: &mut HookCx<'_>) {
        if self.day.as_ref().is_some_and(|d| d.date == date) {
            return;
        }
        if let Err(e) = self.save(cx) {
            cx.diagnostic(format!("cannot save usage day ({e})"));
        }
        let loaded = match cx.get(date) {
            Ok(Some(bytes)) => match serde_json::from_slice::<Day>(&bytes) {
                Ok(day) => Some(day),
                Err(e) => {
                    cx.diagnostic(format!("usage {date} unusable ({e}); counting from zero"));
                    None
                }
            },
            Ok(None) => None,
            Err(e) => {
                cx.diagnostic(format!(
                    "cannot read usage {date} ({e}); counting from zero"
                ));
                None
            }
        };
        self.day = Some(match loaded {
            Some(day) => day,
            None => Day {
                version: 1,
                date: date.to_owned(),
                ..Day::default()
            },
        });
        self.dirty = false;
    }

    fn publish(&self, cx: &mut HookCx<'_>) {
        if let Some(day) = &self.day
            && let Ok(json) = serde_json::to_string(day)
        {
            cx.summary(json);
        }
    }
}

impl Hook for UsageHook {
    fn name(&self) -> &'static str {
        "usage"
    }

    fn on_record(&mut self, rec: &Record, cx: &mut HookCx<'_>) {
        if !matches!(rec.event, Event::SessionStart { .. } | Event::Clock { .. })
            && self.anchor.is_none()
            && let Some((wall, offset)) = self.pending
        {
            self.anchor = Some((rec.t, wall, offset));
            // A re-anchor can move to another date (sleep across midnight,
            // new offset): save the old day and publish the new one.
            if let Some(date) = self.date_at(rec.t)
                && self.day.as_ref().is_some_and(|d| d.date != date)
            {
                self.switch_to(&date, cx);
                self.publish(cx);
            }
        }
        match &rec.event {
            Event::SessionStart {
                wall_unix_ms,
                utc_offset_min,
                ..
            } => {
                self.pending = Some((*wall_unix_ms, *utc_offset_min));
                self.anchor = None;
            }
            Event::Clock {
                wall_unix_ms,
                utc_offset_min,
            } => {
                // The uptime clock of `t` stops while the Mac sleeps and the
                // offset may change (travel, DST). The record carries the
                // last event time, which may lie long before the wall clock
                // was read, so the next record (fresh input, close to the
                // read) re-anchors, as after `session_start`.
                self.pending = Some((*wall_unix_ms, *utc_offset_min));
                self.anchor = None;
            }
            Event::Context { bundle_id, .. } => self.app.clone_from(bundle_id),
            Event::WordCommit { word_id, mode, .. } => {
                let Some(date) = self.date_at(rec.t) else {
                    if !self.warned_no_anchor {
                        self.warned_no_anchor = true;
                        cx.diagnostic("word_commit before session_start; not counted".into());
                    }
                    return;
                };
                self.switch_to(&date, cx);
                let Some(day) = self.day.as_mut() else {
                    return;
                };
                // Before any context the app is unknown (""): counted in
                // the total only.
                let entry = Some(&self.app)
                    .filter(|a| !a.is_empty())
                    .map(|a| day.apps.entry(a.clone()).or_default());
                for counts in entry.into_iter().chain([&mut day.total]) {
                    match mode {
                        Lang::Vi => counts.vi += 1,
                        Lang::En => counts.en += 1,
                    }
                }
                if self.word_app.len() >= WORD_APPS {
                    self.word_app.pop_front();
                }
                self.word_app.push_back((*word_id, self.app.clone()));
                self.dirty = true;
                self.publish(cx);
            }
            Event::Label {
                target: Target::Word(id),
                source: LabelSource::Correction,
                ..
            } => {
                let app = self
                    .word_app
                    .iter()
                    .rev()
                    .find(|(w, _)| w == id)
                    .map_or_else(|| self.app.clone(), |(_, app)| app.clone());
                let Some(day) = self.day.as_mut() else {
                    return;
                };
                if !app.is_empty() {
                    day.apps.entry(app).or_default().corrected += 1;
                }
                day.total.corrected += 1;
                self.dirty = true;
                self.publish(cx);
            }
            _ => {}
        }
    }

    fn flush(&mut self, cx: &mut HookCx<'_>) -> io::Result<()> {
        self.save(cx)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use nimk_storage::{MemStorage, Storage};
    use nimk_types::{Boundary, Event, LabelSource, Lang, Record, Target};

    fn rec(seq: u64, t: f64, event: Event) -> Record {
        Record {
            v: 1,
            seq,
            t,
            session: "s".into(),
            event,
        }
    }
    fn start(wall_ms: u64, offset: i32) -> Event {
        Event::SessionStart {
            wall_unix_ms: wall_ms,
            app_version: "a".into(),
            config_json: "{}".into(),
            policy_id: "p".into(),
            utc_offset_min: offset,
        }
    }
    fn ctx(app: &str) -> Event {
        Event::Context {
            bundle_id: app.into(),
            field_id: 1,
            caret: None,
        }
    }
    fn commit(id: u64, mode: Lang) -> Event {
        Event::WordCommit {
            word_id: id,
            raw: "x".into(),
            vi: "x".into(),
            en: "x".into(),
            mode,
            boundary: Boundary::Space,
        }
    }

    fn feed(events: Vec<(f64, Event)>) -> MemStorage {
        let s = MemStorage::new();
        let mut out = Vec::new();
        let mut h = UsageHook::new();
        h.init(&mut HookCx::new("usage", s.blobs(), &mut out));
        for (i, (t, e)) in events.into_iter().enumerate() {
            h.on_record(
                &rec(i as u64, t, e),
                &mut HookCx::new("usage", s.blobs(), &mut out),
            );
        }
        h.flush(&mut HookCx::new("usage", s.blobs(), &mut out))
            .unwrap();
        s
    }

    fn day(s: &MemStorage, d: &str) -> serde_json::Value {
        serde_json::from_slice(&s.blobs().get(&format!("hooks/usage/{d}")).unwrap().unwrap())
            .unwrap()
    }

    #[test]
    fn counts_words_per_app_and_corrections() {
        // 2026-11-09T10:00:00Z
        let s = feed(vec![
            (100.0, start(1_794_218_400_000, 0)),
            (100.0, ctx("com.apple.Terminal")),
            (101.0, commit(0, Lang::En)),
            (102.0, commit(1, Lang::Vi)),
            (
                103.0,
                Event::Label {
                    target: Target::Word(1),
                    label: Lang::En,
                    weight: 1.0,
                    source: LabelSource::Correction,
                },
            ),
            (
                104.0,
                Event::Label {
                    target: Target::Word(0),
                    label: Lang::En,
                    weight: 0.3,
                    source: LabelSource::Implicit,
                },
            ),
        ]);
        let d = day(&s, "2026-11-09");
        assert_eq!(d["apps"]["com.apple.Terminal"]["en"], 1);
        assert_eq!(d["apps"]["com.apple.Terminal"]["vi"], 1);
        assert_eq!(d["apps"]["com.apple.Terminal"]["corrected"], 1);
        assert_eq!(d["total"]["corrected"], 1);
    }

    #[test]
    fn day_bucket_uses_local_offset() {
        // 2026-11-09T23:30:00Z with +07:00 is 2026-11-10 local.
        let s = feed(vec![
            (0.0, start(1_794_267_000_000, 420)),
            (0.0, ctx("a")),
            (1.0, commit(0, Lang::Vi)),
        ]);
        assert_eq!(day(&s, "2026-11-10")["total"]["vi"], 1);
    }

    #[test]
    fn day_rolls_over_during_a_session() {
        // 23:59:59Z, then two seconds later: next day.
        let s = feed(vec![
            (0.0, start(1_794_268_799_000, 0)),
            (0.0, ctx("a")),
            (0.5, commit(0, Lang::Vi)),
            (2.0, commit(1, Lang::En)),
        ]);
        assert_eq!(day(&s, "2026-11-09")["total"]["vi"], 1);
        assert_eq!(day(&s, "2026-11-10")["total"]["en"], 1);
    }

    #[test]
    fn anchors_on_first_record_after_session_start_not_its_t() {
        // session_start is logged at t = 0; later records carry uptime.
        // 2026-11-09T10:00:00Z; +14h reaches local midnight at offset 0.
        let s = feed(vec![
            (0.0, start(1_794_218_400_000, 0)),
            (500_000.0, ctx("a")),
            (500_001.0, commit(0, Lang::Vi)),
            (500_000.0 + 14.0 * 3600.0 + 1.0, commit(1, Lang::En)),
        ]);
        assert_eq!(day(&s, "2026-11-09")["total"]["vi"], 1);
        assert_eq!(day(&s, "2026-11-10")["total"]["en"], 1);
    }

    #[test]
    fn clock_record_re_anchors_after_sleep() {
        // 2026-11-09T10:00:00Z; the Mac sleeps a day: uptime advances only
        // 10 s, but the wall clock read on wake is a day later.
        let s = feed(vec![
            (0.0, start(1_794_218_400_000, 0)),
            (100.0, ctx("a")),
            (101.0, commit(0, Lang::Vi)),
            (
                105.0,
                Event::Clock {
                    wall_unix_ms: 1_794_218_400_000 + 86_400_000,
                    utc_offset_min: 0,
                },
            ),
            (106.0, commit(1, Lang::En)),
        ]);
        assert_eq!(day(&s, "2026-11-09")["total"]["vi"], 1);
        assert_eq!(day(&s, "2026-11-09")["total"]["en"], 0);
        assert_eq!(day(&s, "2026-11-10")["total"]["en"], 1);
    }

    #[test]
    fn clock_record_anchors_on_the_next_record_not_its_own_t() {
        // 22:00Z; the Mac stays awake but idle for 1.5 h, then the IME is
        // activated: the clock record carries the last event time (100) and
        // the wall clock 23:30Z. The next key is at uptime 5501: still the
        // 9th. Anchoring at the clock record's own t would give 01:00 on
        // the 10th.
        let s = feed(vec![
            (0.0, start(1_794_261_600_000, 0)),
            (100.0, ctx("a")),
            (100.0, commit(0, Lang::Vi)),
            (
                100.0,
                Event::Clock {
                    wall_unix_ms: 1_794_267_000_000,
                    utc_offset_min: 0,
                },
            ),
            (5501.0, commit(1, Lang::En)),
        ]);
        let d = day(&s, "2026-11-09");
        assert_eq!(d["total"]["vi"], 1);
        assert_eq!(d["total"]["en"], 1);
    }

    #[test]
    fn clock_record_takes_the_new_utc_offset() {
        // 2026-11-09T20:00:00Z at offset 0, then travel to +07:00: the
        // same instant is 2026-11-10 local.
        let s = feed(vec![
            (0.0, start(1_794_254_400_000, 0)),
            (1.0, ctx("a")),
            (2.0, commit(0, Lang::Vi)),
            (
                3.0,
                Event::Clock {
                    wall_unix_ms: 1_794_254_401_000,
                    utc_offset_min: 420,
                },
            ),
            (4.0, commit(1, Lang::En)),
        ]);
        assert_eq!(day(&s, "2026-11-09")["total"]["vi"], 1);
        assert_eq!(day(&s, "2026-11-10")["total"]["en"], 1);
    }

    #[test]
    fn commit_before_any_context_counts_in_total_only() {
        let s = feed(vec![
            (0.0, start(1_794_218_400_000, 0)),
            (1.0, commit(0, Lang::Vi)),
            (
                2.0,
                Event::Label {
                    target: Target::Word(0),
                    label: Lang::En,
                    weight: 1.0,
                    source: LabelSource::Correction,
                },
            ),
            (3.0, ctx("a")),
            (4.0, commit(1, Lang::En)),
        ]);
        let d = day(&s, "2026-11-09");
        assert_eq!(d["total"]["vi"], 1);
        assert_eq!(d["total"]["en"], 1);
        assert_eq!(d["total"]["corrected"], 1);
        assert!(d["apps"].get("").is_none(), "{d}");
        assert_eq!(d["apps"]["a"]["en"], 1);
    }

    #[test]
    fn existing_day_is_extended_not_overwritten() {
        let s = feed(vec![
            (0.0, start(1_794_218_400_000, 0)),
            (0.0, ctx("a")),
            (1.0, commit(0, Lang::Vi)),
        ]);
        let mut out = Vec::new();
        let mut h = UsageHook::new();
        h.init(&mut HookCx::new("usage", s.blobs(), &mut out));
        for (i, e) in [start(1_794_218_500_000, 0), ctx("a"), commit(0, Lang::Vi)]
            .into_iter()
            .enumerate()
        {
            h.on_record(
                &rec(i as u64, 0.0, e),
                &mut HookCx::new("usage", s.blobs(), &mut out),
            );
        }
        h.flush(&mut HookCx::new("usage", s.blobs(), &mut out))
            .unwrap();
        assert_eq!(day(&s, "2026-11-09")["total"]["vi"], 2);
    }

    #[test]
    fn commit_without_session_start_is_ignored_once() {
        let s = MemStorage::new();
        let mut out = Vec::new();
        let mut h = UsageHook::new();
        for i in 0..2 {
            h.on_record(
                &rec(i, 0.0, commit(i, Lang::Vi)),
                &mut HookCx::new("usage", s.blobs(), &mut out),
            );
        }
        assert_eq!(out.len(), 1);
    }
}
