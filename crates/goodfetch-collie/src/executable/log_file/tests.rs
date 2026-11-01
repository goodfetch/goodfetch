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
use std::time::{Duration, SystemTime};

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

/// Makes the empty file `name` in `folder`, last written `age` seconds ago.
fn aged(folder: &Path, name: &str, age: u64) {
    let file = std::fs::File::create(folder.join(name)).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(age))
        .unwrap();
}

/// A process other than the Daemon writes a set of its own,
/// `<executable>-<process ID>.jsonl` and its rotated files. At its start
/// the newest `max_files` sets of its executable are kept, its own
/// included, by when each set was last written, and the rest go, so the
/// folder stays bounded. The Daemon's set, another executable's and any
/// other file are left as they are.
///
/// Covers: row:RT-17, setting:log.file.max_files
#[test]
fn a_process_keeps_the_newest_sets_of_its_executable_at_its_start() {
    let env = TestEnv::new();
    let folder = env.folder().join("logs");
    std::fs::create_dir_all(&folder).unwrap();
    for (name, age) in [
        ("collie-11.jsonl", 500),
        ("collie-11.1.jsonl", 600),
        ("collie-12.jsonl", 100),
        ("collie-12.1.jsonl", 200),
        ("collie-13.jsonl", 400),
        ("collie-14.1.jsonl", 50),
        ("collie-14.jsonl", 300),
        ("daemon-0.0.jsonl", 900),
        ("collie-cli-15.jsonl", 900),
        ("notes.txt", 900),
    ] {
        aged(&folder, name, age);
    }

    let log = LogFile::open_of_process(
        &folder,
        "collie",
        99,
        Rotation {
            max_size: 100,
            max_files: 3,
        },
        Clock::system(),
    )
    .unwrap();
    written(&log, 1, 30);

    let names: Vec<String> = files(&folder).into_iter().map(|(name, _)| name).collect();
    assert_eq!(
        names,
        [
            "collie-12.1.jsonl",
            "collie-12.jsonl",
            "collie-14.1.jsonl",
            "collie-14.jsonl",
            "collie-99.jsonl",
            "collie-cli-15.jsonl",
            "daemon-0.0.jsonl",
            "notes.txt",
        ]
    );
}
