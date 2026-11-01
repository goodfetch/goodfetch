//! The engine process's own log files (ADR 0122): JSON lines in the
//! Goodfetch home's log folder, `<stem>.jsonl`, rotated by size. A file
//! rotates when the next line would take it past `log.file.max_size`, to
//! `<stem>.1.jsonl` and on, and `log.file.max_files` are kept, the one
//! written included, so a file holds at most its size, or one line when a
//! line is larger. A file opened again goes on from its size. A process
//! other than the Daemon writes a set of its own, `<executable>-<process
//! ID>.jsonl`, and at its start keeps only its executable's newest
//! `log.file.max_files` sets.
//!
//! The writer is lossy and never blocks: each line goes to a queue of its
//! own thread, which writes it. A line that finds the queue full, or that
//! the disk refuses, is dropped and counted, and the next line written is
//! preceded by one that says how many were dropped. Logs are telemetry,
//! never slowing a run (ADR 0078).
#![expect(
    clippy::disallowed_methods,
    reason = "the log files are written on the log writer's own thread, never on an async worker, which starts before the runtime and outlives it as the OTLP export's batch thread does (ADR 0122)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "one counter of dropped lines, which writers add to as they drop and the writer's thread takes, with nothing else under it: no lock order for loom to test (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::time::{Duration, SystemTime};

use goodfetch_engine::clock::Clock;

/// How many lines wait for the writer's thread before more are dropped.
const QUEUE: usize = 4096;

/// How long a flush waits for the writer's thread, at most.
const FLUSH_DEADLINE: Duration = Duration::from_secs(2);

/// When a log file rotates, and how many are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Rotation {
    /// `log.file.max_size`, in bytes.
    pub(super) max_size: u64,
    /// `log.file.max_files`, the file written included.
    pub(super) max_files: u64,
}

/// What the writer's thread is handed.
enum Message {
    /// A line, its newline included.
    Line(Vec<u8>),
    /// Answer once every line before is written.
    Flush(SyncSender<()>),
}

/// A process's log file, which any `tracing` layer writes through.
#[derive(Clone, Debug)]
pub(super) struct LogFile {
    queue: SyncSender<Message>,
    dropped: Arc<AtomicU64>,
}

impl LogFile {
    /// Opens the log file `<stem>.jsonl` in `folder`, which it creates,
    /// rotated as `rotation` says, and starts its writer's thread; the
    /// line that counts dropped lines takes its time from `clock`.
    ///
    /// # Errors
    ///
    /// When the folder or the file cannot be made or opened, or the thread
    /// cannot start.
    pub(super) fn open(
        folder: &Path,
        stem: &str,
        rotation: Rotation,
        clock: Clock,
    ) -> io::Result<Self> {
        let mut files = Files::open(folder, stem, rotation)?;
        let (queue, lines) = mpsc::sync_channel(QUEUE);
        let dropped = Arc::new(AtomicU64::new(0));
        let counted = Arc::clone(&dropped);
        std::thread::Builder::new()
            .name("log-file".to_owned())
            .spawn(move || {
                for message in lines {
                    match message {
                        Message::Line(line) => files.write_counting(&line, &counted, clock),
                        Message::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            })?;
        Ok(Self { queue, dropped })
    }

    /// Opens the log file of `executable`'s process `process`, a process
    /// other than the Daemon, `<executable>-<process>.jsonl` in `folder`,
    /// a set of its own, as [`LogFile::open`] does; then keeps the newest
    /// `max_files` sets of `executable`, its own included, by when each set
    /// was last written, and removes the older ones (ADR 0122), so the
    /// folder stays bounded. A file it cannot remove waits for the next
    /// start.
    ///
    /// # Errors
    ///
    /// As [`LogFile::open`].
    pub(super) fn open_of_process(
        folder: &Path,
        executable: &str,
        process: u32,
        rotation: Rotation,
        clock: Clock,
    ) -> io::Result<Self> {
        let stem = format!("{executable}-{process}");
        let opened = Self::open(folder, &stem, rotation, clock)?;
        keep_newest_sets(folder, executable, &stem, rotation.max_files.max(1));
        Ok(opened)
    }

    /// Waits until every line handed over before is written, at most a
    /// moment.
    pub(super) fn flush(&self) {
        let (done, flushed) = mpsc::sync_channel(1);
        if self.queue.send(Message::Flush(done)).is_ok() {
            let _ = flushed.recv_timeout(FLUSH_DEADLINE);
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogFile {
    type Writer = Line<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        Line(self)
    }
}

/// One `tracing` event's line on its way to the log file: each write is a
/// whole line, as the formatting layers write one event at once.
pub(super) struct Line<'a>(&'a LogFile);

impl io::Write for Line<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self.0.queue.try_send(Message::Line(bytes.to_vec())) {
            Ok(()) => {}
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.0.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The rotated files of one stem, on the writer's thread.
struct Files {
    folder: PathBuf,
    stem: String,
    rotation: Rotation,
    file: File,
    size: u64,
}

impl Files {
    fn open(folder: &Path, stem: &str, rotation: Rotation) -> io::Result<Self> {
        std::fs::create_dir_all(folder)?;
        let mut files = Self {
            folder: folder.to_owned(),
            stem: stem.to_owned(),
            rotation,
            file: File::options()
                .create(true)
                .append(true)
                .open(folder.join(format!("{stem}.jsonl")))?,
            size: 0,
        };
        files.size = files.file.metadata()?.len();
        Ok(files)
    }

    /// The file numbered `index`, 0 the one written.
    fn path(&self, index: u64) -> PathBuf {
        match index {
            0 => self.folder.join(format!("{}.jsonl", self.stem)),
            _ => self.folder.join(format!("{}.{index}.jsonl", self.stem)),
        }
    }

    /// Writes `line`, first saying how many lines `dropped` counted; a line
    /// the disk refuses is counted there instead.
    fn write_counting(&mut self, line: &[u8], dropped: &AtomicU64, clock: Clock) {
        let lost = dropped.swap(0, Ordering::Relaxed);
        if lost > 0 && self.write(&notice(lost, clock)).is_err() {
            dropped.fetch_add(lost + 1, Ordering::Relaxed);
            return;
        }
        if self.write(line).is_err() {
            dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Writes `line`, rotating first when it would take the file past its
    /// size; a line the disk refuses leaves the file as it was.
    fn write(&mut self, line: &[u8]) -> io::Result<()> {
        let length = u64::try_from(line.len()).unwrap_or(u64::MAX);
        if self.size > 0 && self.size.saturating_add(length) > self.rotation.max_size {
            self.rotate()?;
        }
        match self.file.write_all(line) {
            Ok(()) => {
                self.size += length;
                Ok(())
            }
            Err(error) => {
                // No torn line stays: the next one starts a line of its own.
                let _ = self.file.set_len(self.size);
                Err(error)
            }
        }
    }

    /// Moves each file one number up, the last kept one's away, and starts
    /// the file of the plain name again.
    fn rotate(&mut self) -> io::Result<()> {
        let kept = self.rotation.max_files.max(1);
        let _ = std::fs::remove_file(self.path(kept - 1));
        for index in (0..kept - 1).rev() {
            match std::fs::rename(self.path(index), self.path(index + 1)) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
        }
        self.file = File::options()
            .create(true)
            .append(true)
            .open(self.path(0))?;
        self.size = self.file.metadata()?.len();
        Ok(())
    }
}

/// Keeps the newest `keep` sets of `executable`'s per-process files in
/// `folder`, `own` included whenever it was written, and removes the
/// files of the others; a set is as new as its last written file.
fn keep_newest_sets(folder: &Path, executable: &str, own: &str, keep: u64) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut sets: BTreeMap<String, (SystemTime, Vec<PathBuf>)> = BTreeMap::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(set) = name.to_str().and_then(|name| set_of(name, executable)) else {
            continue;
        };
        if set == own {
            continue;
        }
        let written = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let (newest, paths) = sets
            .entry(set.to_owned())
            .or_insert((SystemTime::UNIX_EPOCH, Vec::new()));
        *newest = (*newest).max(written);
        paths.push(entry.path());
    }
    let mut sets: Vec<(SystemTime, Vec<PathBuf>)> = sets.into_values().collect();
    sets.sort_by_key(|(newest, _)| std::cmp::Reverse(*newest));
    let others = usize::try_from(keep.saturating_sub(1)).unwrap_or(usize::MAX);
    for path in sets.into_iter().skip(others).flat_map(|(_, paths)| paths) {
        // Another process starting at once may have removed it already.
        let _ = std::fs::remove_file(path);
    }
}

/// The set `name` belongs to, `<executable>-<process ID>`, when it is one
/// of `executable`'s per-process files: `<executable>-<process ID>.jsonl`
/// or a rotated `<executable>-<process ID>.<n>.jsonl`.
fn set_of<'a>(name: &'a str, executable: &str) -> Option<&'a str> {
    let rest = name
        .strip_prefix(executable)?
        .strip_prefix('-')?
        .strip_suffix(".jsonl")?;
    let (process, index) = rest.split_once('.').unwrap_or((rest, "1"));
    let number = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    (number(process) && number(index)).then(|| &name[..executable.len() + 1 + process.len()])
}

/// The line that says `count` lines were dropped, at `clock`'s time.
fn notice(count: u64, clock: Clock) -> Vec<u8> {
    let timestamp = jiff::Timestamp::try_from(clock.wall())
        .map(|time| time.to_string())
        .unwrap_or_default();
    let mut line = serde_json::to_vec(&serde_json::json!({
        "timestamp": timestamp,
        "level": "WARN",
        "fields": {
            "message": "log lines were dropped",
            "dropped": count,
        },
        "target": "goodfetch::log",
    }))
    .expect("invariant: a notice encodes");
    line.push(b'\n');
    line
}

#[cfg(test)]
mod tests;
