//! Behaviour shared by every `Storage` backend, run against both.
#![allow(clippy::unwrap_used)]

use std::io;

use nimk_storage::{MemStorage, PlainFsStorage, SegmentId, Storage};

fn read_all(storage: &dyn Storage, id: &SegmentId) -> Vec<Vec<u8>> {
    let source = storage.event_source().unwrap();
    source
        .read_segment(id)
        .unwrap()
        .collect::<io::Result<_>>()
        .unwrap()
}

fn segments(storage: &dyn Storage) -> Vec<SegmentId> {
    storage.event_source().unwrap().segments().unwrap()
}

/// Runs `check` against a fresh FS storage and a fresh memory storage.
fn for_each_backend(check: impl Fn(&dyn Storage)) {
    let dir = tempfile::tempdir().unwrap();
    check(&PlainFsStorage::open(dir.path().join("root")).unwrap());
    check(&MemStorage::new());
}

#[test]
fn append_flush_read_round_trip() {
    for_each_backend(|s| {
        let mut sink = s.event_sink("sess1").unwrap();
        sink.append(br#"{"seq":0}"#).unwrap();
        sink.append(b"").unwrap();
        sink.append(br#"{"seq":2}"#).unwrap();
        sink.flush().unwrap();
        drop(sink);
        let ids = segments(s);
        assert_eq!(ids.len(), 1);
        assert!(ids[0].0.ends_with("_sess1"), "{:?}", ids[0]);
        assert_eq!(
            read_all(s, &ids[0]),
            vec![
                br#"{"seq":0}"#.to_vec(),
                Vec::new(),
                br#"{"seq":2}"#.to_vec()
            ]
        );
    });
}

#[test]
fn multiple_sessions_give_ordered_segments() {
    // FS orders same-day segments by session id; memory by creation order.
    // Session ids chosen so both orders agree.
    for_each_backend(|s| {
        for (i, session) in ["a-1", "b-2", "c-3"].iter().enumerate() {
            let mut sink = s.event_sink(session).unwrap();
            sink.append(format!("r{i}").as_bytes()).unwrap();
            sink.flush().unwrap();
        }
        let ids = segments(s);
        assert_eq!(ids.len(), 3);
        for (i, (id, session)) in ids.iter().zip(["a-1", "b-2", "c-3"]).enumerate() {
            assert!(id.0.ends_with(&format!("_{session}")), "{id:?}");
            assert_eq!(read_all(s, id), vec![format!("r{i}").into_bytes()]);
        }
    });
}

#[test]
fn memory_segments_follow_creation_order() {
    let s = MemStorage::new();
    for session in ["zz", "aa", "mm"] {
        drop(s.event_sink(session).unwrap());
    }
    let ids: Vec<String> = segments(&s).into_iter().map(|i| i.0).collect();
    assert_eq!(ids, ["0000000001_zz", "0000000002_aa", "0000000003_mm"]);
}

#[test]
fn session_is_sanitized_and_empty_rejected() {
    for_each_backend(|s| {
        drop(s.event_sink("../evil id").unwrap());
        let ids = segments(s);
        assert!(ids[0].0.ends_with("____evil_id"), "{:?}", ids[0]);
        let err = s.event_sink("").err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    });
}

#[test]
fn newline_in_record_is_rejected() {
    for_each_backend(|s| {
        let mut sink = s.event_sink("s").unwrap();
        let err = sink.append(b"a\nb").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        sink.append(b"ok").unwrap();
        sink.flush().unwrap();
        let ids = segments(s);
        assert_eq!(read_all(s, &ids[0]), vec![b"ok".to_vec()]);
    });
}

#[test]
fn read_while_sink_open_after_flush() {
    for_each_backend(|s| {
        let mut sink = s.event_sink("live").unwrap();
        sink.append(b"one").unwrap();
        sink.flush().unwrap();
        let ids = segments(s);
        assert_eq!(read_all(s, &ids[0]), vec![b"one".to_vec()]);
        sink.append(b"two").unwrap();
        sink.flush().unwrap();
        assert_eq!(read_all(s, &ids[0]), vec![b"one".to_vec(), b"two".to_vec()]);
    });
}

#[test]
fn bad_segment_ids_are_rejected() {
    for_each_backend(|s| {
        let source = s.event_source().unwrap();
        let err = source
            .read_segment(&SegmentId("../x".into()))
            .err()
            .unwrap();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        let err = source
            .read_segment(&SegmentId("missing".into()))
            .err()
            .unwrap();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    });
}

#[test]
fn blob_overwrite_get_and_list() {
    for_each_backend(|s| {
        let b = s.blobs();
        assert_eq!(b.get("model/tiny_mlp/latest").unwrap(), None);
        b.put("model/tiny_mlp/latest", b"v1").unwrap();
        b.put("model/tiny_mlp/latest", b"v2").unwrap();
        b.put("model/linear/latest", b"l").unwrap();
        b.put("config", b"{}").unwrap();
        assert_eq!(
            b.get("model/tiny_mlp/latest").unwrap(),
            Some(b"v2".to_vec())
        );
        assert_eq!(b.get("model").unwrap(), None);
        assert_eq!(
            b.list("model/").unwrap(),
            ["model/linear/latest", "model/tiny_mlp/latest"]
        );
        assert_eq!(
            b.list("").unwrap(),
            ["config", "model/linear/latest", "model/tiny_mlp/latest"]
        );
        assert!(b.list("nope").unwrap().is_empty());
    });
}

#[test]
fn bad_blob_keys_are_rejected() {
    for_each_backend(|s| {
        let b = s.blobs();
        for key in ["", "/etc/passwd", "a//b", "a/", "../x", "a/../b", ".tmp"] {
            assert_eq!(
                b.put(key, b"x").unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "put {key:?}"
            );
            assert_eq!(
                b.get(key).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "get {key:?}"
            );
        }
        assert!(b.list("").unwrap().is_empty());
    });
}

#[test]
fn fs_blob_put_leaves_no_temp_files() {
    let dir = tempfile::tempdir().unwrap();
    let s = PlainFsStorage::open(dir.path()).unwrap();
    s.blobs().put("m/k", b"1").unwrap();
    s.blobs().put("m/k", b"2").unwrap();
    // arch-lint: allow(no-sync-io) reason="test inspects files directly"
    let names: Vec<_> = std::fs::read_dir(dir.path().join("blobs/m"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["k"]);
}

#[test]
fn fs_layout_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    {
        let s = PlainFsStorage::open(dir.path()).unwrap();
        let mut sink = s.event_sink("x").unwrap();
        sink.append(b"r").unwrap();
        // Dropped without explicit flush: Drop flushes best-effort.
    }
    assert!(dir.path().join("blobs").is_dir());
    let s = PlainFsStorage::open(dir.path()).unwrap();
    let ids = segments(&s);
    assert_eq!(ids.len(), 1);
    let file = dir
        .path()
        .join("events")
        .join(format!("{}.jsonl", ids[0].0));
    assert_eq!(std::fs::read(file).unwrap(), b"r\n");
    assert_eq!(read_all(&s, &ids[0]), vec![b"r".to_vec()]);
}

#[test]
fn memory_clones_share_data() {
    let a = MemStorage::new();
    let b = a.clone();
    let mut sink = a.event_sink("s").unwrap();
    sink.append(b"r").unwrap();
    a.blobs().put("k", b"v").unwrap();
    let ids = segments(&b);
    assert_eq!(read_all(&b, &ids[0]), vec![b"r".to_vec()]);
    assert_eq!(b.blobs().get("k").unwrap(), Some(b"v".to_vec()));
    drop(b.event_sink("t").unwrap());
    assert_eq!(segments(&a).len(), 2);
}

#[test]
fn delete_removes_blob_and_reports_it() {
    for_each_backend(|s| {
        let b = s.blobs();
        b.put("model/m/latest", b"x").unwrap();
        assert!(b.delete("model/m/latest").unwrap());
        assert_eq!(b.get("model/m/latest").unwrap(), None);
        assert!(!b.delete("model/m/latest").unwrap());
        assert_eq!(
            b.delete("../x").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    });
}
