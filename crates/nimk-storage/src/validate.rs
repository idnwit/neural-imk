//! Input validation shared by every backend, so file and memory storage
//! accept and reject exactly the same names.

use std::io;

/// Builds an `InvalidInput` error with a static message.
pub(crate) fn invalid(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}

/// Returns `true` for characters allowed in session and segment names.
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Maps a session id to `[A-Za-z0-9_-]+`, replacing other characters with `_`.
///
/// # Errors
/// Returns `InvalidInput` if `session` is empty.
pub(crate) fn sanitize_session(session: &str) -> io::Result<String> {
    if session.is_empty() {
        return Err(invalid("session id must not be empty"));
    }
    Ok(session
        .chars()
        .map(|c| if is_name_char(c) { c } else { '_' })
        .collect())
}

/// Checks that a segment id is a plain `[A-Za-z0-9_-]+` name (no paths).
///
/// # Errors
/// Returns `InvalidInput` otherwise.
pub(crate) fn check_segment_id(id: &str) -> io::Result<()> {
    if !id.is_empty() && id.chars().all(is_name_char) {
        Ok(())
    } else {
        Err(invalid("segment id must match [A-Za-z0-9_-]+"))
    }
}

/// Checks that a record can be stored as one line.
///
/// # Errors
/// Returns `InvalidInput` if `record` contains `\n`.
pub(crate) fn check_record(record: &[u8]) -> io::Result<()> {
    if record.contains(&b'\n') {
        Err(invalid("record must not contain '\\n'"))
    } else {
        Ok(())
    }
}

/// Validates a blob key and returns its `/`-separated segments.
///
/// A key is one or more segments joined by `/`. Each segment must be
/// non-empty, must not start with `.` (rules out `.`, `..` and the hidden
/// temp files used by atomic writes) and must not contain `\` or NUL.
/// A leading `/` (absolute path) yields an empty first segment and is rejected.
///
/// # Errors
/// Returns `InvalidInput` for keys that break these rules.
pub(crate) fn blob_key_segments(key: &str) -> io::Result<Vec<&str>> {
    if key.is_empty() {
        return Err(invalid("blob key must not be empty"));
    }
    let segments: Vec<&str> = key.split('/').collect();
    for seg in &segments {
        if seg.is_empty() {
            return Err(invalid("blob key has an empty segment or is absolute"));
        }
        if seg.starts_with('.') {
            return Err(invalid("blob key segment must not start with '.'"));
        }
        if seg.contains(['\\', '\0']) {
            return Err(invalid("blob key must not contain '\\' or NUL"));
        }
    }
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_disallowed_chars() {
        assert_eq!(sanitize_session("a b/../c-1_x").unwrap(), "a_b____c-1_x");
        assert!(sanitize_session("").is_err());
    }

    #[test]
    fn blob_keys() {
        assert_eq!(
            blob_key_segments("model/tiny_mlp/latest").unwrap(),
            vec!["model", "tiny_mlp", "latest"]
        );
        for bad in [
            "", "/abs", "a//b", "a/", "..", "a/../b", "./a", ".hidden", "a\\b",
        ] {
            assert!(
                blob_key_segments(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn segment_ids() {
        assert!(check_segment_id("2026-11-08_abc").is_ok());
        assert!(check_segment_id("../x").is_err());
        assert!(check_segment_id("").is_err());
    }
}
