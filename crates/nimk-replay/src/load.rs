//! Reads event log segments and groups their records into sessions.

use std::collections::HashMap;
use std::io;

use nimk_storage::EventSource;
use nimk_types::{Record, SCHEMA_VERSION};

/// All records of one logged session, sorted by `seq`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Session {
    /// Session id from the records.
    pub(crate) id: String,
    /// Records in `seq` order.
    pub(crate) records: Vec<Record>,
}

/// Parsed log.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct LoadedLog {
    /// Sessions in order of first appearance (segments sort chronologically).
    pub(crate) sessions: Vec<Session>,
    /// Segments read.
    pub(crate) segments: usize,
    /// Records parsed successfully.
    pub(crate) records: usize,
    /// Lines skipped: invalid JSON, unknown event type or schema version.
    pub(crate) malformed: usize,
    /// Human-readable notes about skipped lines.
    pub(crate) warnings: Vec<String>,
}

/// Parses one log line, rejecting other schema versions.
fn parse(line: &[u8]) -> Result<Record, String> {
    let record: Record = serde_json::from_slice(line).map_err(|e| e.to_string())?;
    if record.v != SCHEMA_VERSION {
        return Err(format!(
            "schema version {} (expected {SCHEMA_VERSION})",
            record.v
        ));
    }
    Ok(record)
}

/// Reads every segment whose id sorts at or after `from`.
///
/// # Errors
/// Fails if a segment cannot be listed or read; malformed lines are counted
/// instead.
pub(crate) fn load(source: &dyn EventSource, from: Option<&str>) -> io::Result<LoadedLog> {
    let mut log = LoadedLog::default();
    let mut index: HashMap<String, usize> = HashMap::new();
    for id in source.segments()? {
        if from.is_some_and(|f| id.0.as_str() < f) {
            continue;
        }
        log.segments += 1;
        for (n, line) in source.read_segment(&id)?.enumerate() {
            let line = line?;
            let record = match parse(&line) {
                Ok(r) => r,
                Err(e) => {
                    log.malformed += 1;
                    log.warnings
                        .push(format!("{} line {}: skipped ({e})", id.0, n + 1));
                    continue;
                }
            };
            log.records += 1;
            let slot = *index.entry(record.session.clone()).or_insert_with(|| {
                log.sessions.push(Session {
                    id: record.session.clone(),
                    records: Vec::new(),
                });
                log.sessions.len() - 1
            });
            if let Some(s) = log.sessions.get_mut(slot) {
                s.records.push(record);
            }
        }
    }
    for s in &mut log.sessions {
        s.records.sort_by_key(|r| r.seq);
    }
    Ok(log)
}

#[cfg(test)]
mod tests {
    use nimk_storage::{MemStorage, Storage};

    use super::*;

    fn line(session: &str, seq: u64) -> String {
        format!(
            r#"{{"v":1,"seq":{seq},"t":1.0,"session":"{session}","type":"context","bundle_id":"b","field_id":1,"caret":null}}"#
        )
    }

    #[test]
    fn groups_sorts_and_counts_malformed() {
        let storage = MemStorage::new();
        let mut a = storage.event_sink("a").unwrap();
        a.append(line("a", 1).as_bytes()).unwrap();
        a.append(b"{not json").unwrap();
        a.append(line("a", 0).as_bytes()).unwrap();
        let mut b = storage.event_sink("b").unwrap();
        b.append(line("b", 0).as_bytes()).unwrap();
        b.append(line("a", 2).replace("\"v\":1", "\"v\":9").as_bytes())
            .unwrap();

        let source = storage.event_source().unwrap();
        let log = load(source.as_ref(), None).unwrap();
        assert_eq!(log.segments, 2);
        assert_eq!(log.records, 3);
        assert_eq!(log.malformed, 2);
        let ids: Vec<_> = log.sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
        let seqs: Vec<_> = log.sessions[0].records.iter().map(|r| r.seq).collect();
        assert_eq!(seqs, [0, 1]);
    }

    #[test]
    fn from_skips_earlier_segments() {
        let storage = MemStorage::new();
        storage
            .event_sink("a")
            .unwrap()
            .append(line("a", 0).as_bytes())
            .unwrap();
        storage
            .event_sink("b")
            .unwrap()
            .append(line("b", 0).as_bytes())
            .unwrap();
        let source = storage.event_source().unwrap();
        let log = load(source.as_ref(), Some("0000000002")).unwrap();
        assert_eq!(log.segments, 1);
        assert_eq!(log.sessions.len(), 1);
        assert_eq!(log.sessions[0].id, "b");
    }
}
