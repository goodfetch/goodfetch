//! The log file writer through its interface (spec #213's Seam 4, the
//! executable crate's composition in process): lines in, the files of the
//! log folder out (ADR 0122).
#![expect(
    clippy::disallowed_methods,
    reason = "a test reads the files the writer wrote, outside any async worker"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io::Write as _;
use std::path::Path;

use goodfetch_engine::clock::Clock;
use goodfetch_test_support::TestEnv;
use tracing_subscriber::fmt::MakeWriter as _;

use super::{LogFile, Rotation};

/// The log files in `folder`, by name, each with its lines.
fn files(folder: &Path) -> Vec<(String, Vec<String>)> {
    let mut files: Vec<(String, Vec<String>)> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let lines = std::fs::read_to_string(&path)
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect();
            (name, lines)
        })
        .collect();
    files.sort();
    files
}

/// Writes `count` lines `line <n>` of `width` bytes each, newline included.
fn written(log: &LogFile, count: usize, width: usize) {
    for line in 0..count {
        let mut text = format!("line {line:02} ");
        text.push_str(&"x".repeat(width - text.len() - 1));
        text.push('\n');
        log.make_writer().write_all(text.as_bytes()).unwrap();
    }
    log.flush();
}

/// A file rotates once the next line would take it past `max_size`, and
/// `max_files` are kept, the one written included: the newest lines in the
/// file of the plain name, the oldest files gone.
///
/// Covers: row:RT-17, setting:log.file.max_size, setting:log.file.max_files
#[test]
fn a_log_file_rotates_at_its_size_and_keeps_its_count_of_files() {
    let env = TestEnv::new();
    let folder = env.folder().join("logs");
    let log = LogFile::open(
        &folder,
        "daemon-0.0",
        Rotation {
            max_size: 100,
            max_files: 3,
        },
        Clock::system(),
    )
    .unwrap();

    written(&log, 11, 30);

    let found = files(&folder);
    let names: Vec<&str> = found.iter().map(|(name, _)| name.as_str()).collect();
    let firsts: Vec<&str> = found.iter().map(|(_, lines)| &lines[0][..7]).collect();
    let sizes: Vec<usize> = found
        .iter()
        .map(|(_, lines)| lines.iter().map(|line| line.len() + 1).sum())
        .collect();
    assert_eq!(
        (names, firsts, sizes),
        (
            vec![
                "daemon-0.0.1.jsonl",
                "daemon-0.0.2.jsonl",
                "daemon-0.0.jsonl"
            ],
            vec!["line 06", "line 03", "line 09"],
            vec![90, 90, 60],
        )
    );
}

/// A file opened again goes on from its size, so a Daemon started again
/// keeps one set of files.
///
/// Covers: row:RT-17, setting:log.file.max_size
#[test]
fn a_log_file_opened_again_goes_on_from_its_size() {
    let env = TestEnv::new();
    let folder = env.folder().join("logs");
    let rotation = Rotation {
        max_size: 100,
        max_files: 2,
    };
    written(
        &LogFile::open(&folder, "daemon-0.0", rotation, Clock::system()).unwrap(),
        2,
        30,
    );

    written(
        &LogFile::open(&folder, "daemon-0.0", rotation, Clock::system()).unwrap(),
        2,
        30,
    );

    let found = files(&folder);
    let counts: Vec<(&str, usize)> = found
        .iter()
        .map(|(name, lines)| (name.as_str(), lines.len()))
        .collect();
    assert_eq!(
        counts,
        vec![("daemon-0.0.1.jsonl", 3), ("daemon-0.0.jsonl", 1)]
    );
}
