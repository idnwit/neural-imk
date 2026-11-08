//! Event sink of [`crate::PlainFsStorage`]: buffered JSONL writer that rotates
//! to a new segment file when the UTC date changes.

use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::EventSink;
use crate::validate::{check_record, sanitize_session};

/// Seconds per day; UTC days ignore leap seconds like Unix time does.
const SECS_PER_DAY: u64 = 86_400;

/// Clock returning the current UTC day number (days since 1970-01-01).
pub(crate) type DayClock = Box<dyn FnMut() -> io::Result<u64> + Send>;

/// Current UTC day number from the wall clock.
///
/// # Errors
/// Returns an error if the system clock is before the Unix epoch.
pub(crate) fn utc_day_now() -> io::Result<u64> {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| io::Error::other("system clock is before 1970-01-01"))?;
    Ok(since.as_secs() / SECS_PER_DAY)
}

/// Formats a day number as `YYYY-MM-DD` (proleptic Gregorian, UTC).
///
/// Uses Howard Hinnant's `civil_from_days`, restricted to days >= 0.
pub(crate) fn format_day(days: u64) -> String {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Path of the segment file for `day` and `session`.
pub(crate) fn segment_path(dir: &Path, day: u64, session: &str) -> PathBuf {
    dir.join(format!("{}_{session}.jsonl", format_day(day)))
}

/// Opens a segment for appending. If a previous writer crashed mid-line,
/// a `\n` is added first so the next record does not merge with the
/// truncated one.
fn open_segment(path: &Path) -> io::Result<BufWriter<File>> {
    let mut file = File::options()
        .read(true)
        .append(true)
        .create(true)
        .open(path)?;
    if file.seek(SeekFrom::End(0))? > 0 {
        let mut last = [0u8; 1];
        file.seek(SeekFrom::End(-1))?;
        file.read_exact(&mut last)?;
        if last[0] != b'\n' {
            file.write_all(b"\n")?;
        }
    }
    Ok(BufWriter::new(file))
}

/// Buffered, day-rotating JSONL writer for one session.
///
/// Two sinks for the same session on the same day append to the same file;
/// callers should use unique session ids.
pub(crate) struct FsEventSink {
    dir: PathBuf,
    session: String,
    clock: DayClock,
    day: u64,
    writer: BufWriter<File>,
}

impl FsEventSink {
    /// Opens the sink using the wall clock.
    ///
    /// # Errors
    /// Returns an error if `session` is empty or the file cannot be opened.
    pub(crate) fn open(dir: PathBuf, session: &str) -> io::Result<Self> {
        Self::with_clock(dir, session, Box::new(utc_day_now))
    }

    /// Opens the sink with an injected clock (tests rotation).
    ///
    /// # Errors
    /// Returns an error if `session` is empty or the file cannot be opened.
    pub(crate) fn with_clock(dir: PathBuf, session: &str, mut clock: DayClock) -> io::Result<Self> {
        let session = sanitize_session(session)?;
        let day = clock()?;
        let writer = open_segment(&segment_path(&dir, day, &session))?;
        Ok(Self {
            dir,
            session,
            clock,
            day,
            writer,
        })
    }

    /// Flushes the buffer and fsyncs the current file.
    fn sync(&mut self) -> io::Result<()> {
        self.writer.flush()?;
        self.writer.get_ref().sync_data()
    }
}

impl EventSink for FsEventSink {
    fn append(&mut self, record: &[u8]) -> io::Result<()> {
        check_record(record)?;
        let day = (self.clock)()?;
        if day != self.day {
            self.sync()?;
            self.writer = open_segment(&segment_path(&self.dir, day, &self.session))?;
            self.day = day;
        }
        self.writer.write_all(record)?;
        self.writer.write_all(b"\n")
    }

    fn flush(&mut self) -> io::Result<()> {
        self.sync()
    }
}

impl Drop for FsEventSink {
    fn drop(&mut self) {
        // Best effort: errors cannot be reported from drop.
        // arch-lint: allow(no-silent-result-drop) reason="Drop cannot report errors; call flush() to observe them"
        let _ = self.writer.flush();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    #[test]
    fn formats_known_dates() {
        assert_eq!(format_day(0), "1970-01-01");
        assert_eq!(format_day(59), "1970-03-01");
        assert_eq!(format_day(11_016), "2000-02-29");
        assert_eq!(format_day(20_400), "2025-11-08");
        assert_eq!(format_day(20_765), "2026-11-08");
    }

    #[test]
    fn rotates_when_day_changes() {
        let dir = tempfile::tempdir().unwrap();
        let day = Arc::new(AtomicU64::new(20_765));
        let clock_day = Arc::clone(&day);
        let clock: DayClock = Box::new(move || Ok(clock_day.load(Ordering::SeqCst)));
        let mut sink = FsEventSink::with_clock(dir.path().to_path_buf(), "s", clock).unwrap();
        sink.append(b"a").unwrap();
        day.store(20_766, Ordering::SeqCst);
        sink.append(b"b").unwrap();
        sink.flush().unwrap();
        // arch-lint: allow(no-sync-io) reason="test inspects files directly"
        let first = std::fs::read(dir.path().join("2026-11-08_s.jsonl")).unwrap();
        // arch-lint: allow(no-sync-io) reason="test inspects files directly"
        let second = std::fs::read(dir.path().join("2026-11-09_s.jsonl")).unwrap();
        assert_eq!(first, b"a\n");
        assert_eq!(second, b"b\n");
    }

    #[test]
    fn reopen_terminates_truncated_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = segment_path(dir.path(), 20_765, "s");
        // arch-lint: allow(no-sync-io) reason="test inspects files directly"
        std::fs::write(&path, b"ok\npart").unwrap();
        let clock: DayClock = Box::new(|| Ok(20_765));
        let mut sink = FsEventSink::with_clock(dir.path().to_path_buf(), "s", clock).unwrap();
        sink.append(b"next").unwrap();
        sink.flush().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"ok\npart\nnext\n");
    }
}
