//! The Storage conformance cases' Collie lines (TESTING.md; spec #213's
//! Seam 1; ADR 0122), on Collie's executable over its protocol, attached
//! and through a Daemon (`daemon run`), on a tmpfs of its own holding the
//! Goodfetch home and the run folder, with `storage.low_free` and
//! `storage.critical_free` set against its size and a filler file to cross
//! them; the home's `logs` folder stays on the test's disk, so the engine's
//! JSON log reaches the exercise record:
//! - **Low:** the Run goes on, its progress reads `storage` at `low` with
//!   its folder, freeing space brings it back to `normal`, and its report
//!   carries `storage_low`.
//! - **Critical:** the Run stops at its next safe point, `stopped` and
//!   `incomplete`, with `stopReason` `resources`, `resource: "storage"` and
//!   its folder, no Checkpoint copy written; a new Run's start there raises
//!   `resources_exhausted`, not retryable, whose `fix` names the folder;
//!   `run.checkpoint` fails with `file_write_failed`, side `engine`,
//!   writing nothing; and once space is freed a resume goes on.
//! - **A failed write:** with both levels off, a commit the full disk
//!   refuses stops the Run; the commit recording the stop fails too, so its
//!   waiting `run.finish` raises `resources_exhausted`, and once space is
//!   freed a resume goes on as after a crash.
//! - **`/readyz` and the metrics:** through a Daemon with its ops listener,
//!   `/readyz` fails while the run folder's filesystem is at `critical`,
//!   not at `low`, and succeeds once space is freed; the storage metrics
//!   read its level, its free space and its free inodes, labelled by what
//!   it holds. In the per-install layout, with the run folders on a tmpfs
//!   of their own inside the home's, `/readyz` and `holds="run_state"`
//!   follow the install database's filesystem, the home's.
//! - **A read-only location:** `run.checkpoint` fails with
//!   `file_write_failed`, naming `EROFS`, and leaves no file.
//!
//! The crawl and WARC lines are area 6's.
#![expect(
    clippy::disallowed_methods,
    reason = "a test reads the Daemon's token through its mount namespace, outside any async worker"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::home::DaemonFile;
use goodfetch_test_client::DaemonClient;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{MESSAGE_DEADLINE, TestEnv, browser_cache};
use rstest::rstest;
use serde_json::{Value, json};

use crate::clean_up_rules::calendar_free;
use crate::daemon_hello;
use crate::daemon_run::{ANY_PORT, Foreground};
use crate::ops::{get, series};
use goodfetch_test_client::{Attached, hello};
use goodfetch_test_support::{
    fill_filesystem, free_filesystem, inside_mount, read_only_mounted, tmpfs_mounted_home,
};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

const MIB: u64 = 1 << 20;

/// The tmpfs's size, and the two levels against it.
const SIZE: u64 = 64 * MIB;
const LEVELS: &str = "storage.low_free = 41943040\nstorage.critical_free = 20971520\n\
                      governor.sample_interval = 100\n";

/// Free space between the two levels, and below `critical`.
const LOW_ROOM: u64 = 30 * MIB;
const CRITICAL_ROOM: u64 = 10 * MIB;

/// How the test reaches the engine.
#[derive(Clone, Copy, Debug)]
enum Mode {
    Attached,
    Daemon,
}

/// The engine of a test, on its tmpfs.
enum Engine {
    Attached(Attached),
    Daemon {
        daemon: Foreground,
        client: DaemonClient,
    },
}

impl Engine {
    /// Starts Collie's executable in `mode` with a tmpfs over the test's
    /// home, its user configuration holding `settings`, and opens the
    /// Connection subscribed to every event.
    fn start(env: &TestEnv, mode: Mode, settings: &str) -> Self {
        let mut engine = match mode {
            Mode::Attached => {
                env.user_configuration(settings);
                let mut command = tmpfs_mounted_home(env, SIZE, COLLIE);
                command.arg("attached-engine");
                let mut engine =
                    Self::Attached(Attached::spawned(env, MESSAGE_DEADLINE, &mut command));
                engine.send(&hello(Product::Collie));
                let _ = engine.next();
                engine
            }
            Mode::Daemon => {
                env.user_configuration(&format!("{settings}daemon.address = \"{ANY_PORT}\"\n"));
                let mut command = tmpfs_mounted_home(env, SIZE, COLLIE);
                command.args(["daemon", "run"]);
                let mut daemon = Foreground::spawned(env, &mut command);
                let endpoint = daemon.serving();
                let pid = daemon.process.child().id();
                let token = std::fs::read_to_string(inside_mount(
                    pid,
                    &HomeLayout::of(env.home()).daemon_file(DaemonFile::Token),
                ))
                .unwrap();
                let client = DaemonClient::opened(&endpoint, &daemon_hello(&token));
                Self::Daemon { daemon, client }
            }
        };
        engine.send(
            &json!({ "jsonrpc": "2.0", "id": 0, "method": "events.subscribe", "params": {} }),
        );
        engine.send(&json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": 0, "credits": 10_000 } }));
        engine
    }

    fn pid(&self) -> u32 {
        match self {
            Self::Attached(engine) => engine.pid(),
            Self::Daemon { daemon, .. } => daemon.process.child().id(),
        }
    }

    fn send(&mut self, message: &Value) {
        match self {
            Self::Attached(engine) => engine.send(message),
            Self::Daemon { client, .. } => client.try_send(message),
        }
    }

    fn next(&mut self) -> Option<Value> {
        match self {
            Self::Attached(engine) => engine.next(),
            Self::Daemon { client, .. } => client.next(),
        }
    }

    /// Sends the call `id` and answers its reply, past the events on the
    /// way.
    fn call(&mut self, id: i64, method: &str, params: &Value) -> Value {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let message = self.next().expect("the engine closed before replying");
            if message["id"] == json!(id) {
                return message;
            }
        }
    }

    /// The `storage` of the next `run.progress` at `level`.
    fn progress_at(&mut self, level: &str) -> Value {
        loop {
            let message = self.next().expect("the engine closed before its progress");
            let events = message["params"]["events"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if let Some(event) = events.iter().find(|event| {
                event["event"] == "run.progress" && event["data"]["storage"]["level"] == level
            }) {
                return event["data"]["storage"].clone();
            }
        }
    }

    /// Ends the engine and its tmpfs.
    fn stop(self) {
        match self {
            Self::Attached(engine) => {
                let _ = engine.finish();
            }
            Self::Daemon { mut daemon, client } => {
                drop(client);
                daemon.terminate();
            }
        }
    }
}

fn run_folder(env: &TestEnv, run_id: &str) -> PathBuf {
    HomeLayout::of(env.home()).run_folder(run_id)
}

/// The codes of `report`'s warnings, but the calendar's
/// ([`calendar_free`]).
fn warnings(report: &Value) -> Vec<Value> {
    report["warnings"]
        .as_array()
        .map(|warnings| {
            calendar_free(warnings)
                .iter()
                .map(|warning| warning["code"].clone())
                .collect()
        })
        .unwrap_or_default()
}

fn exists(pid: u32, path: &Path) -> bool {
    inside_mount(pid, path).exists()
}

/// Covers: story:3-30, error:file_write_failed
#[test]
fn a_checkpoint_to_a_read_only_filesystem_fails_and_leaves_no_file() {
    let env = TestEnv::new();
    let read_only = env.folder().join("read-only");
    let mut command = read_only_mounted(&env, &read_only, COLLIE);
    command.arg("attached-engine");
    let mut engine = Attached::spawned(&env, MESSAGE_DEADLINE, &mut command);
    let pid = engine.pid();
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    let started = engine.ask(1, "run.start", &json!({ "run_id": "refused" }));

    let failed = engine.ask(
        2,
        "run.checkpoint",
        &json!({
            "run_id": "refused",
            "location": read_only.join("kept.sqlite").display().to_string(),
        }),
    );

    let left = std::fs::read_dir(inside_mount(pid, &read_only))
        .unwrap()
        .count();
    let _ = engine.finish();
    let mut data = failed["error"]["data"].clone();
    data["path"].take();
    assert_eq!(
        (started.get("result").is_some(), data, left),
        (
            true,
            json!({
                "code": "file_write_failed",
                "retryable": false,
                "reason": "read_only_filesystem",
                "os_code": "EROFS",
                "side": "engine",
                "path": null,
            }),
            0,
        )
    );
}

/// Low: crossing `storage.low_free` leaves the Run working, its progress
/// reads `storage` at `low` with its folder, freeing space brings it back
/// to `normal`, and its report carries `storage_low`.
///
/// Covers: row:RT-03, setting:storage.low_free, event:run.progress
#[rstest]
#[case::attached(Mode::Attached)]
#[case::through_a_daemon(Mode::Daemon)]
fn past_low_free_a_run_goes_on_and_carries_storage_low(#[case] mode: Mode) {
    let env = TestEnv::new();
    let mut engine = Engine::start(&env, mode, LEVELS);
    let pid = engine.pid();

    let started = engine.call(1, "run.start", &json!({ "run_id": "low" }));
    let filler = fill_filesystem(pid, env.home(), LOW_ROOM);
    let low = engine.progress_at("low");
    free_filesystem(&filler);
    let normal = engine.progress_at("normal");
    let finished = engine.call(2, "run.finish", &json!({ "run_id": "low" }));
    engine.stop();

    let folder = json!(run_folder(&env, "low").display().to_string());
    assert_eq!(
        (
            started.get("result").is_some(),
            &low["folder"],
            &normal["level"],
            &finished["result"]["lifecycle"],
            warnings(&finished["result"]),
        ),
        (
            true,
            &folder,
            &json!("normal"),
            &json!("finished"),
            vec![json!("storage_low")],
        ),
        "{finished}"
    );
}

/// Critical: crossing `storage.critical_free` stops the Run resumably,
/// writing no Checkpoint copy there; a start there raises
/// `resources_exhausted`, and `run.checkpoint` fails with
/// `file_write_failed`, writing nothing; once space is freed the Run
/// resumes and finishes.
///
/// Covers: row:RT-03, setting:storage.critical_free, error:resources_exhausted, error:file_write_failed, operation:run.resume
#[rstest]
#[case::attached(Mode::Attached)]
#[case::through_a_daemon(Mode::Daemon)]
fn past_critical_free_a_run_stops_resumably_and_nothing_new_is_written_there(#[case] mode: Mode) {
    let env = TestEnv::new();
    let mut engine = Engine::start(&env, mode, LEVELS);
    let pid = engine.pid();

    engine.call(1, "run.start", &json!({ "run_id": "full" }));
    let filler = fill_filesystem(pid, env.home(), CRITICAL_ROOM);
    engine.progress_at("critical");
    let stopped = engine.call(2, "run.finish", &json!({ "run_id": "full" }));
    let refused = engine.call(3, "run.start", &json!({ "run_id": "another" }));
    let checkpoint = run_folder(&env, "full").join("copy.sqlite");
    let copied = engine.call(
        4,
        "run.checkpoint",
        &json!({ "run_id": "full", "location": checkpoint.display().to_string() }),
    );
    let written = (
        exists(pid, &checkpoint),
        exists(pid, &run_folder(&env, "full").join("checkpoint.sqlite")),
        exists(pid, &run_folder(&env, "another")),
    );
    free_filesystem(&filler);
    let resumed = engine.call(5, "run.resume", &json!({ "run_id": "full" }));
    let finished = engine.call(6, "run.finish", &json!({ "run_id": "full" }));
    engine.stop();

    let folder = run_folder(&env, "full").display().to_string();
    let runs = HomeLayout::of(env.home()).runs().display().to_string();
    assert_eq!(
        (
            (
                &stopped["result"]["lifecycle"],
                &stopped["result"]["completeness"],
                &stopped["result"]["stop_reason"],
                &stopped["result"]["checkpoint"],
            ),
            (
                &refused["error"]["data"]["code"],
                &refused["error"]["data"]["retryable"],
                refused["error"]["data"]["fix"]
                    .as_str()
                    .is_some_and(|fix| fix.contains(&runs)),
            ),
            (
                &copied["error"]["data"]["code"],
                &copied["error"]["data"]["side"],
                &copied["error"]["data"]["reason"],
            ),
            written,
            (
                &resumed["result"]["started_as"],
                &finished["result"]["lifecycle"]
            ),
        ),
        (
            (
                &json!("stopped"),
                &json!("incomplete"),
                &json!({ "kind": "resources", "resource": "storage", "folder": folder }),
                &Value::Null,
            ),
            (&json!("resources_exhausted"), &json!(false), true),
            (
                &json!("file_write_failed"),
                &json!("engine"),
                &json!("storage_full")
            ),
            (false, false, false),
            (&json!("resumed"), &json!("finished")),
        ),
        "{stopped}\n{refused}\n{copied}"
    );
}

/// A failed write: with both levels off, a commit the full disk refuses
/// stops the Run, the commit recording the stop fails too, so the waiting
/// `run.finish` raises `resources_exhausted`; once space is freed the Run
/// resumes as after a crash and finishes.
///
/// Covers: row:RT-03, error:resources_exhausted, operation:run.set_progress_message, operation:run.resume
#[rstest]
#[case::attached(Mode::Attached)]
#[case::through_a_daemon(Mode::Daemon)]
fn a_write_the_full_disk_refuses_stops_the_run_and_its_waiting_call_raises_resources_exhausted(
    #[case] mode: Mode,
) {
    let env = TestEnv::new();
    let mut engine = Engine::start(
        &env,
        mode,
        "storage.low_free = 0\nstorage.critical_free = 0\n",
    );
    let pid = engine.pid();

    engine.call(1, "run.start", &json!({ "run_id": "refused" }));
    let filler = fill_filesystem(pid, env.home(), 0);
    let failed = engine.call(
        2,
        "run.set_progress_message",
        &json!({ "run_id": "refused", "message": "a status line the full disk refuses" }),
    );
    let stopped = engine.call(3, "run.finish", &json!({ "run_id": "refused" }));
    free_filesystem(&filler);
    let resumed = engine.call(4, "run.resume", &json!({ "run_id": "refused" }));
    let finished = engine.call(5, "run.finish", &json!({ "run_id": "refused" }));
    engine.stop();

    assert_eq!(
        (
            failed.get("error").is_some(),
            &stopped["error"]["data"]["code"],
            &stopped["error"]["data"]["resource"],
            &resumed["result"]["started_as"],
            &finished["result"]["lifecycle"],
        ),
        (
            true,
            &json!("resources_exhausted"),
            &json!("storage"),
            &json!("resumed"),
            &json!("finished"),
        ),
        "{failed}\n{stopped}"
    );
}

/// How long a test waits for the storage metrics to read what it set; a
/// failure, never a measure.
const METRICS_DEADLINE: Duration = Duration::from_secs(30);

/// The storage series of a scrape: each value by the series' name and its
/// `holds` label.
type Storage = BTreeMap<(String, String), String>;

/// A foreground Daemon on a tmpfs over the test's home, as
/// [`Engine::start`]'s, with its ops listener on a free port and the
/// pinned browser installed, so that only storage keeps it from being
/// ready; and where its ops listener listens, and its ops token.
fn daemon_with_ops(env: &TestEnv) -> (Foreground, SocketAddr, String) {
    env.user_configuration(&format!(
        "{LEVELS}daemon.address = \"{ANY_PORT}\"\nops.address = \"127.0.0.1:0\"\n"
    ));
    let mut command = tmpfs_mounted_home(env, SIZE, COLLIE);
    command.args(["daemon", "run"]);
    daemon_from(env, command)
}

/// A tmpfs of its own over the run folders, inside the home's tmpfs, in
/// the same mount namespace: `RUNS_SIZE` bytes, more than the levels
/// keep, so the run folders stay at `normal` while the home fills.
const OWN_RUNS: &str = "set -e; mkdir -p \"$1\"; mount -t tmpfs -o \"size=$2\" tmpfs \"$1\"; \
    shift 2; exec \"$@\"";

/// The run folders' tmpfs's size.
const RUNS_SIZE: u64 = 256 * MIB;

/// [`daemon_with_ops`], in the per-install layout, its run folders on a
/// tmpfs of their own while the install database stays on the home's.
fn per_install_daemon_with_ops(env: &TestEnv) -> (Foreground, SocketAddr, String) {
    env.user_configuration(&format!(
        "{LEVELS}daemon.address = \"{ANY_PORT}\"\nops.address = \"127.0.0.1:0\"\n\
         run_state.layout = \"per_install\"\n"
    ));
    let mut command = tmpfs_mounted_home(env, SIZE, "sh");
    command
        .args(["-c", OWN_RUNS, "sh"])
        .arg(HomeLayout::of(env.home()).runs())
        .arg(RUNS_SIZE.to_string())
        .args([COLLIE, "daemon", "run"]);
    daemon_from(env, command)
}

/// The foreground Daemon `command` starts, with the pinned browser
/// installed; where its ops listener listens, and its ops token.
fn daemon_from(env: &TestEnv, mut command: Command) -> (Foreground, SocketAddr, String) {
    command.env("GOODFETCH_BROWSERS_PATH", browser_cache(env).unwrap());
    let mut daemon = Foreground::spawned(env, &mut command);
    let address = daemon.record("the ops listener serves")["address"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let pid = daemon.process.child().id();
    let token = std::fs::read_to_string(inside_mount(
        pid,
        &HomeLayout::of(env.home()).daemon_file(DaemonFile::OpsToken),
    ))
    .unwrap();
    (daemon, address, token)
}

/// What the filesystem holding `path` has left to an unprivileged user, as
/// `stat --file-system` reads it: its bytes and its inodes.
fn left(path: &Path) -> (String, String) {
    let output = Command::new("stat")
        .args(["--file-system", "--format", "%a %S %d"])
        .arg(path)
        .output()
        .unwrap();
    let read = String::from_utf8(output.stdout).unwrap();
    let fields: Vec<u64> = read
        .split_whitespace()
        .map(|field| field.parse().unwrap())
        .collect();
    ((fields[0] * fields[1]).to_string(), fields[2].to_string())
}

/// The storage series of `/metrics` at `address`.
fn scraped(address: SocketAddr, token: &str) -> Storage {
    let (_, text) = get(address, "/metrics", Some(token));
    series(&text)
        .into_iter()
        .filter(|(name, _, _)| name.starts_with("goodfetch_storage_"))
        .map(|(name, labels, value)| {
            let holds = labels.get("holds").cloned().unwrap_or_default();
            ((name, holds), value)
        })
        .collect()
}

/// The storage series once run state's read `level` and what the
/// filesystem holding `folder`, as `pid` sees it, has left; fails the test
/// past the deadline with the last series read.
fn storage_at(address: SocketAddr, token: &str, pid: u32, folder: &Path, level: &str) -> Storage {
    let deadline = Instant::now() + METRICS_DEADLINE;
    loop {
        let read = scraped(address, token);
        let (bytes, inodes) = left(&inside_mount(pid, folder));
        let of = |name: &str| {
            read.get(&(name.to_owned(), "run_state".to_owned()))
                .cloned()
        };
        if (
            of("goodfetch_storage_level"),
            of("goodfetch_storage_available_bytes"),
            of("goodfetch_storage_inodes_available"),
        ) == (Some(level.to_owned()), Some(bytes), Some(inodes))
        {
            return read;
        }
        assert!(
            Instant::now() < deadline,
            "the storage metrics never read level {level} and what the filesystem has left: {read:#?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The `holds` label of each series `name` of `read`.
fn holds(read: &Storage, name: &str) -> Vec<String> {
    read.keys()
        .filter(|(series, _)| series == name)
        .map(|(_, holds)| holds.clone())
        .collect()
}

/// `/readyz` and the metrics: through a Daemon with its ops listener,
/// `/readyz` answers 200 at `normal` and at `low`, fails at `critical`, and
/// answers 200 again once space is freed; the storage metrics give run
/// state's filesystem its level (0, 1, 2), its free space and its free
/// inodes, the Goodfetch home on the same filesystem the same, and the
/// temporary folder's filesystem a series of its own.
///
/// Covers: story:13-45, story:13-46, setting:storage.critical_free, setting:ops.address
#[test]
fn readyz_fails_while_run_states_filesystem_is_critical_and_the_metrics_read_its_level() {
    let env = TestEnv::new();
    let (mut daemon, address, token) = daemon_with_ops(&env);
    let pid = daemon.process.child().id();

    let normal = storage_at(address, &token, pid, env.home(), "0");
    let ready_at_normal = get(address, "/readyz", None).0;
    let filler = fill_filesystem(pid, env.home(), LOW_ROOM);
    storage_at(address, &token, pid, env.home(), "1");
    let ready_at_low = get(address, "/readyz", None).0;
    free_filesystem(&filler);
    let filler = fill_filesystem(pid, env.home(), CRITICAL_ROOM);
    let critical = storage_at(address, &token, pid, env.home(), "2");
    let ready_at_critical = get(address, "/readyz", None).0;
    free_filesystem(&filler);
    storage_at(address, &token, pid, env.home(), "0");
    let ready_once_freed = get(address, "/readyz", None).0;
    daemon.terminate();

    let same = |name: &str| {
        critical.get(&(name.to_owned(), "home".to_owned()))
            == critical.get(&(name.to_owned(), "run_state".to_owned()))
    };
    let every = vec!["home".to_owned(), "run_state".to_owned(), "temp".to_owned()];
    assert_eq!(
        (
            (
                ready_at_normal,
                ready_at_low,
                ready_at_critical,
                ready_once_freed
            ),
            [
                holds(&normal, "goodfetch_storage_available_bytes"),
                holds(&normal, "goodfetch_storage_inodes_available"),
                holds(&normal, "goodfetch_storage_level"),
            ],
            [
                same("goodfetch_storage_available_bytes"),
                same("goodfetch_storage_inodes_available"),
                same("goodfetch_storage_level"),
            ],
        ),
        (
            (200, 200, 503, 200),
            [every.clone(), every.clone(), every],
            [true; 3],
        ),
        "{critical:#?}"
    );
}

/// `/readyz` and the metrics in the per-install layout, its run folders on
/// a tmpfs of their own: run state is the install database, in the
/// per-major folder on the home's filesystem, so `/readyz` fails and
/// `holds="run_state"` reads `critical` while that filesystem is at
/// `critical`, though the run folders' stays at `normal`, and `/readyz`
/// succeeds again once space is freed.
///
/// Covers: story:13-45, story:13-46, setting:storage.critical_free, setting:run_state.layout
#[test]
fn in_the_per_install_layout_readyz_follows_the_install_databases_filesystem() {
    let env = TestEnv::new();
    let (mut daemon, address, token) = per_install_daemon_with_ops(&env);
    let pid = daemon.process.child().id();

    storage_at(address, &token, pid, env.home(), "0");
    let ready_at_normal = get(address, "/readyz", None).0;
    let filler = fill_filesystem(pid, env.home(), CRITICAL_ROOM);
    let critical = storage_at(address, &token, pid, env.home(), "2");
    let ready_at_critical = get(address, "/readyz", None).0;
    let runs_left = left(&inside_mount(pid, &HomeLayout::of(env.home()).runs()));
    let home_left = left(&inside_mount(pid, env.home()));
    free_filesystem(&filler);
    storage_at(address, &token, pid, env.home(), "0");
    let ready_once_freed = get(address, "/readyz", None).0;
    daemon.terminate();

    assert_eq!(
        (
            (ready_at_normal, ready_at_critical, ready_once_freed),
            critical.get(&("goodfetch_storage_level".to_owned(), "home".to_owned())),
            runs_left.0 != home_left.0,
        ),
        ((200, 503, 200), Some(&"2".to_owned()), true),
        "{critical:#?}"
    );
}
