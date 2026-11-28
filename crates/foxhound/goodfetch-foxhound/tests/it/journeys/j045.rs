//! J-045, "Deliver a crawl's Dataset to SQLite, kill the Run mid-delivery, and
//! find every Record once", on the `cli` surface (`J-045@foxhound-cli`;
//! ADRs 0026, 0063, 0075 and 0094; issue 1204): a person starts the Daemon
//! with Foxhound's shipped executable, probes a SQLite Sink, starts a
//! code-free crawl delivering its `pages` Dataset to that SQLite Sink, kills
//! the Daemon (`kill -9`) once some Records have landed in SQLite, starts the
//! Daemon again, resumes the crawl to completion, and checks that the probe
//! reported `fenced` and the SQLite destination holds every Record once in
//! order.
#![expect(
    clippy::disallowed_methods,
    reason = "the Journey's CLI steps write settings, poll SQLite and kill the Daemon"
)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{JourneyRun, TestEnv, cli as command_line};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::fetch_tiers::settings;
use crate::journeys::j001::wait_until_stopped;
use crate::journeys::j005::fields_read;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The person's crawl Run ID.
const RUN: &str = "sink-kill-resume";

/// The environment variable holding the SQLite destination database path.
const DESTINATION_ENV: &str = "SINK_J044_DATABASE";

/// How many Records must land in SQLite before the Daemon is killed.
const LANDED_BEFORE_KILL: usize = 2;

/// Writes the crawl settings with a `sqlite` Sink (`warehouse`) and 200 ms
/// domain pacing so the kill lands mid-delivery.
fn sink_crawl_settings(run: &JourneyRun<impl Sized>) -> PathBuf {
    let proving_ground = run.proving_ground().expect("J-045 has a Proving ground");
    let path = run.env().folder().join("sink-settings.json");
    let more = json!({
        "fetch": { "tier": "http" },
        "politeness": { "domain": { "concurrency": 1, "delay": 200 } },
        "sinks": {
            "warehouse": {
                "type": "sqlite",
                "destination": format!("env:{DESTINATION_ENV}"),
                "datasets": ["pages"],
            }
        },
    });
    std::fs::write(&path, settings(proving_ground, &more).to_string()).unwrap();
    path
}

fn daemon_json(env: &TestEnv, engine: &Path, sink_db: &Path, args: &[&str]) -> Value {
    let out = command_line(env, engine)
        .env(DESTINATION_ENV, sink_db)
        .args(args)
        .run()
        .unwrap();
    assert_eq!(out.exit, Some(0), "{out}");
    serde_json::from_str(&out.stdout).expect("CLI printed JSON")
}

fn kill_pid(pid: &Value) {
    let pid = pid.as_u64().expect("the Daemon runs").to_string();
    #[cfg(unix)]
    let status = std::process::Command::new("kill")
        .args(["-KILL", &pid])
        .status()
        .unwrap();
    #[cfg(windows)]
    let status = std::process::Command::new("taskkill")
        .args(["/F", "/PID", &pid])
        .status()
        .unwrap();
    assert!(status.success(), "the Daemon {pid} was not killed");
}

/// Reads `(record_number, url)` rows written to `records` for [`RUN`].
fn sqlite_rows(path: &Path) -> Vec<(u64, String)> {
    let Ok(connection) = Connection::open(path) else {
        return Vec::new();
    };
    let Ok(mut stmt) = connection.prepare(
        "SELECT _goodfetch_record_number, url
         FROM records
         WHERE _goodfetch_run_id = ?1
         ORDER BY _goodfetch_record_number",
    ) else {
        return Vec::new();
    };
    stmt.query_map([RUN], |row| {
        Ok((
            u64::try_from(row.get::<_, i64>(0)?).unwrap_or(0),
            row.get::<_, String>(1)?,
        ))
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

/// Polls `sink_db` until at least `minimum` rows have landed before killing.
fn wait_for_sqlite_rows(sink_db: &Path, minimum: usize) -> Vec<(u64, String)> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let rows = sqlite_rows(sink_db);
        if rows.len() >= minimum {
            return rows;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    sqlite_rows(sink_db)
}

fn start_and_probe(run: &JourneyRun<impl Sized>, sink_db: &Path) -> (Value, PathBuf, Value) {
    let started = daemon_json(
        run.env(),
        run.engine(),
        sink_db,
        &["daemon", "start", "--json"],
    );
    let settings_path = sink_crawl_settings(run);
    let probed = daemon_json(
        run.env(),
        run.engine(),
        sink_db,
        &[
            "sink",
            "probe",
            "--sink",
            "warehouse",
            "--settings",
            settings_path.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(probed["outcome"]["status"], json!("proved"), "{probed}");
    (
        started["pid"].clone(),
        settings_path,
        probed["outcome"]["class"].clone(),
    )
}

fn start_crawl_and_wait(
    run: &JourneyRun<impl Sized>,
    sink_db: &Path,
    settings_path: &Path,
) -> Vec<(u64, String)> {
    let fields = run.env().folder().join("fields.json");
    std::fs::write(&fields, fields_read().to_string()).unwrap();
    let start = run
        .proving_ground()
        .expect("J-045 has a Proving ground")
        .start_url("catalog/1")
        .unwrap();
    let started = command_line(run.env(), run.engine())
        .env(DESTINATION_ENV, sink_db)
        .args(["crawl", "start", &start, "--run-id", RUN])
        .args(["--format", "markdown", "--links", "scope"])
        .args(["--fields", fields.to_str().unwrap()])
        .args(["--settings", settings_path.to_str().unwrap(), "--json"])
        .json("RunId")
        .unwrap();
    assert_eq!(started.documents[0], RUN, "{}", started.output);
    let landed = wait_for_sqlite_rows(sink_db, LANDED_BEFORE_KILL);
    assert!(
        landed.len() >= LANDED_BEFORE_KILL,
        "expected at least {LANDED_BEFORE_KILL} rows in SQLite before kill, got {landed:?}"
    );
    landed
}

fn resume_and_finish(run: &JourneyRun<impl Sized>, sink_db: &Path, settings_path: &Path) -> Value {
    let started = daemon_json(
        run.env(),
        run.engine(),
        sink_db,
        &["daemon", "start", "--json"],
    );
    let resumed = daemon_json(
        run.env(),
        run.engine(),
        sink_db,
        &[
            "run",
            "resume",
            RUN,
            "--settings",
            settings_path.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(resumed["started_as"], json!("resumed"), "{resumed}");
    let followed = command_line(run.env(), run.engine())
        .env(DESTINATION_ENV, sink_db)
        .args(["run", "outcomes", RUN, "--follow", "--json"])
        .run()
        .unwrap();
    assert_eq!(followed.exit, Some(0), "{followed}");
    started["pid"].clone()
}

fn read_committed(run: &JourneyRun<impl Sized>) -> Vec<(u64, String)> {
    let read = command_line(run.env(), run.engine())
        .args(["run", "records", RUN, "--dataset", "pages", "--json"])
        .json_array("RecordRead")
        .unwrap();
    read.documents[0]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["record"]["number"].as_u64().unwrap(),
                entry["record"]["value"]["url"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            )
        })
        .collect()
}

/// Covers: story:6-282, story:6-283
#[test]
fn cli() {
    let mut run = JourneyRun::start("J-045", "cli", Path::new(FOXHOUND), ResultDocument::journey)
        .expect("J-045's cli variant starts");
    run.env()
        .user_configuration("daemon.idle_timeout = 60000\n");
    let sink_db = run.env().folder().join("destination.sqlite");
    let mut settings_path = PathBuf::new();
    let mut pids: Vec<Value> = Vec::new();
    let mut probe_class = Value::Null;
    let mut landed_before_kill = Vec::new();
    let mut report = Value::Null;
    let mut committed: Vec<(u64, String)> = Vec::new();
    let mut landed_after: Vec<(u64, String)> = Vec::new();

    run.step(
        "Start the Daemon and probe the SQLite Sink (`sink probe`)",
        |run| {
            let (pid, path, class) = start_and_probe(run, &sink_db);
            pids.push(pid);
            settings_path = path;
            probe_class = class;
        },
    );
    run.step(
        "Start a Crawl delivering its Dataset to the SQLite Sink and wait until some Records land in SQLite",
        |run| {
            landed_before_kill = start_crawl_and_wait(run, &sink_db, &settings_path);
        },
    );
    run.step("Kill the Daemon (`kill -9`) mid-delivery", |run| {
        let status = command_line(run.env(), run.engine())
            .args(["daemon", "status", "--json"])
            .json("DaemonStatus")
            .unwrap();
        let pid = &status.documents[0]["pid"];
        assert_eq!(pid, &pids[0], "the Daemon the person started still runs");
        kill_pid(pid);
        wait_until_stopped(run.env());
    });
    run.step(
        "Start the Daemon again, resume the Crawl by its Run ID, and wait until it finishes",
        |run| {
            pids.push(resume_and_finish(run, &sink_db, &settings_path));
        },
    );
    run.step(
        "Read the Run report, the Dataset's Records and the SQLite destination's rows",
        |run| {
            report = daemon_json(
                run.env(),
                run.engine(),
                &sink_db,
                &["run", "report", RUN, "--json"],
            );
            committed = read_committed(run);
            landed_after = sqlite_rows(&sink_db);
        },
    );

    run.check(
        "The probe and the Run report both reported the SQLite Sink's Guarantee class (`fenced`)",
        |_| {
            assert_eq!(probe_class, json!("fenced"));
            assert_eq!(
                report["sinks"][0]["guarantee_class"],
                json!("fenced"),
                "{report}"
            );
            assert_eq!(report["sinks"][0]["status"], json!("complete"), "{report}");
        },
    );
    run.check(
        "After the kill and resume, the SQLite destination holds every committed Record of the Dataset exactly once, in order",
        |_| {
            assert!(
                committed.len() > landed_before_kill.len(),
                "expected crawl to commit more Records after resume: before={}, total={}",
                landed_before_kill.len(),
                committed.len()
            );
            assert_ne!(pids[0], pids[1], "Daemon restarted with a new PID");
            let expected_numbers: Vec<u64> = (1..=committed.len() as u64).collect();
            let landed_numbers: Vec<u64> = landed_after.iter().map(|(n, _)| *n).collect();
            assert_eq!(landed_numbers, expected_numbers);
            assert_eq!(landed_after, committed);
        },
    );

    let _ = command_line(run.env(), FOXHOUND)
        .args(["daemon", "stop"])
        .run();
    run.finish()
        .expect("J-045's document is written")
        .assert_proved();
}
