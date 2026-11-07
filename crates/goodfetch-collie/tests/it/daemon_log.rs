//! The Daemon log cases (TESTING.md; spec #213's Seam 1; ADR 0122), on
//! Collie's executable: `daemon run` writes JSON lines to stdout and no log
//! file; a Daemon `daemon start` launched writes them into the Goodfetch
//! home's log folder, rotating at `log.file.max_size` and keeping
//! `log.file.max_files`; at the default level 1,000 requests write no line
//! per request; and on a full filesystem the Daemon keeps serving, and once
//! space returns the next line says how many were dropped.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the Daemon's log files, and wait for its log writer between two moments"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::Path;

use goodfetch_test_client::DaemonClient;
use goodfetch_test_support::{RuntimeFile, TestEnv};
use serde_json::{Value, json};

use crate::daemon::{StopsTheDaemon, json_of};
use crate::daemon_hello;
use crate::daemon_run::{ANY_PORT, Foreground, opened};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// This commit's Product version, whose major folder is `0.0`.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The Daemon's log files in `folder`, by name, with their lines.
fn daemon_files(folder: &Path) -> Vec<(String, Vec<String>)> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut files: Vec<(String, Vec<String>)> = entries
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("daemon-"))
        })
        .map(|path| {
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

/// Starts the test home's Daemon with `daemon start`, and opens a Client of
/// it.
fn started(env: &TestEnv) -> DaemonClient {
    json_of(COLLIE.as_ref(), env, &["daemon", "start", "--json"]);
    let found = RuntimeFile::read(&RuntimeFile::path(env.home(), VERSION)).unwrap();
    DaemonClient::opened(&found.endpoint, &daemon_hello(&found.token))
}

/// `daemon run` keeps ADR 0049's JSON lines on stdout, for its platform to
/// collect and rotate, and writes no log file of its own.
///
/// Covers: row:RT-17, command:daemon.run, setting:log.output
#[test]
fn daemon_run_writes_json_lines_to_stdout_and_no_log_file() {
    let env = TestEnv::new();
    env.user_configuration(&format!(
        "daemon.address = \"{ANY_PORT}\"\nlog.output = \"stdout\"\n"
    ));

    let mut daemon = Foreground::start(&env, COLLIE, &[]);
    let endpoint = daemon.serving();
    let mut client = opened(&env, &endpoint);
    client.call(1, "engine.info", &json!({}));
    drop(client);
    let exited = daemon.terminate();

    let not_json = daemon
        .logged
        .iter()
        .filter(|line| serde_json::from_str::<Value>(line).is_err())
        .count();
    assert_eq!(
        (
            daemon.logged.is_empty(),
            not_json,
            env.home().join("logs").exists(),
            exited
        ),
        (false, 0, false, true)
    );
}

/// A Daemon `daemon start` launched writes JSON lines into the home's log
/// folder, one set of files for the Daemon: past `log.file.max_size` a file
/// rotates, `log.file.max_files` are kept, and each holds at most the size
/// and one line more.
///
/// Covers: row:RT-17, command:daemon.start, setting:log.file.max_size, setting:log.file.max_files
#[test]
fn a_started_daemon_rotates_its_log_files_at_their_size_and_keeps_their_count() {
    let env = TestEnv::new();
    env.user_configuration(
        "daemon.idle_timeout = 60000\nlog.file.max_size = 4000\nlog.file.max_files = 3\n",
    );
    let _stops = StopsTheDaemon::of(&env, COLLIE);

    let mut client = started(&env);
    for id in 1..=300 {
        client.call(id, "engine.info", &json!({}));
    }
    drop(client);
    json_of(COLLIE.as_ref(), &env, &["daemon", "stop", "--json"]);

    let files = daemon_files(&env.home().join("logs"));
    let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
    let oversized: Vec<&str> = files
        .iter()
        .filter(|(_, lines)| {
            let size: usize = lines.iter().map(|line| line.len() + 1).sum();
            let longest = lines.iter().map(|line| line.len() + 1).max().unwrap_or(0);
            size > 4000 + longest
        })
        .map(|(name, _)| name.as_str())
        .collect();
    let not_json = files
        .iter()
        .flat_map(|(_, lines)| lines)
        .filter(|line| serde_json::from_str::<Value>(line).is_err())
        .count();
    assert_eq!(
        (names, oversized, not_json),
        (
            vec![
                "daemon-0.0.1.jsonl",
                "daemon-0.0.2.jsonl",
                "daemon-0.0.jsonl"
            ],
            Vec::<&str>::new(),
            0
        )
    );
}

/// At the default level, `info`, a Daemon's log follows what happens to
/// it, never the number of requests: 1,000 calls write no line each.
///
/// Covers: row:RT-17, setting:log.level
#[test]
fn at_the_default_level_a_thousand_requests_write_no_line_per_request() {
    let env = TestEnv::new();
    env.user_configuration("daemon.idle_timeout = 60000\nlog.level = \"info\"\n");
    let _stops = StopsTheDaemon::of(&env, COLLIE);

    let mut client = started(&env);
    for id in 1..=1000 {
        client.call(id, "engine.info", &json!({}));
    }
    drop(client);
    json_of(COLLIE.as_ref(), &env, &["daemon", "stop", "--json"]);

    let lines: usize = daemon_files(&env.home().join("logs"))
        .iter()
        .map(|(_, lines)| lines.len())
        .sum();
    assert!(
        (1..100).contains(&lines),
        "{lines} lines for 1,000 requests"
    );
}

/// On a full filesystem the Daemon keeps serving, its log lines dropped and
/// counted, and once space returns the next line written says how many were
/// dropped. The log folder is a tmpfs of its own.
///
/// A full tmpfs still takes the bytes that fit in the log file's last page,
/// so a few lines, and a count of the lines dropped before them, may still
/// be written while it is full: how many depends on where the file ended
/// when it filled. Every request's line is therefore either written or
/// counted, and the last count comes before the first line written once
/// space returns.
///
/// Covers: row:RT-17, command:daemon.run, setting:log.output
#[cfg(target_os = "linux")]
#[test]
fn on_a_full_filesystem_the_daemon_keeps_serving_and_then_says_how_many_lines_it_dropped() {
    use std::time::Duration;

    use goodfetch_test_support::{fill_filesystem, free_filesystem, inside_mount, tmpfs_mounted};

    let env = TestEnv::new();
    env.user_configuration(&format!(
        "daemon.address = \"{ANY_PORT}\"\nlog.output = \"file\"\nlog.level = \"trace\"\n"
    ));
    let logs = env.home().join("logs");
    let mut command = tmpfs_mounted(&env, &logs, 1 << 20, COLLIE);
    command.args(["daemon", "run"]);

    let mut daemon = Foreground::spawned(&env, &mut command);
    let endpoint = daemon.serving();
    let pid = daemon.process.child().id();
    let mut client = opened(&env, &endpoint);
    client.call(1, "engine.info", &json!({}));
    let filler = fill_filesystem(pid, &logs, 0);
    let served: Vec<Value> = (2..=21)
        .map(|id| client.call(id, "engine.info", &json!({}))["result"]["product"].clone())
        .collect();
    // The log writer's thread takes the lines the full disk refused.
    std::thread::sleep(Duration::from_millis(500));
    free_filesystem(&filler);
    client.call(22, "engine.info", &json!({}));
    std::thread::sleep(Duration::from_millis(500));
    let written =
        std::fs::read_to_string(inside_mount(pid, &logs.join("daemon-0.0.jsonl"))).unwrap();
    drop(client);
    let exited = daemon.terminate();

    let lines: Vec<Value> = written
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect();
    let dropped: Vec<u64> = lines
        .iter()
        .filter_map(|line| line["fields"]["dropped"].as_u64())
        .collect();
    // Each `engine.info` request logs two lines at `trace`: the dispatch's
    // "request answered" and its span's "close".
    let request_lines = lines
        .iter()
        .filter(|line| {
            line["fields"]["method"] == "engine.info" || line["span"]["rpc.method"] == "engine.info"
        })
        .count();
    let last_count = lines
        .iter()
        .rposition(|line| line["fields"]["dropped"].is_u64());
    let after_space_returned = lines
        .iter()
        .position(|line| line["span"]["jsonrpc.request.id"] == "22");
    assert_eq!(
        (
            served.iter().all(|product| product == "collie"),
            !dropped.is_empty() && dropped.iter().all(|count| *count > 0),
            u64::try_from(request_lines).unwrap() + dropped.iter().sum::<u64>() >= 2 * 22,
            matches!((last_count, after_space_returned), (Some(count), Some(line)) if count < line),
            exited
        ),
        (true, true, true, true, true),
        "{written}"
    );
}
