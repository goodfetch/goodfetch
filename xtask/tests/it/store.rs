//! A fixture results store (ADR 0064) in a workspace folder: one compressed
//! file per run attempt, written in the store's format, for the commands
//! that read the store.

use std::io::Write as _;

use serde_json::{Value, json};

use crate::workspace::Workspace;

/// The store's folder inside the fixture workspace.
pub(crate) const STORE: &str = "test-results";

/// One run attempt of a fixture store.
pub(crate) struct Attempt {
    pub(crate) workflow: &'static str,
    pub(crate) run: u64,
    pub(crate) number: u32,
    /// When the attempt started, RFC 3339.
    pub(crate) started_at: &'static str,
    pub(crate) test_runs: Vec<Value>,
    pub(crate) jobs: Vec<Value>,
    /// The commit the attempt's jobs tested, which each kept artifact's
    /// `context.json` names, and the branch and event of its run.
    pub(crate) commit: &'static str,
    /// The run's head, when it is not the commit its jobs tested, as for a
    /// Release run given a `commit`.
    pub(crate) head: Option<&'static str>,
    pub(crate) branch: Option<&'static str>,
    pub(crate) event: &'static str,
    /// Files kept beside the JUnit files: `(<artifact>/<file>, contents)`,
    /// under `junit/`.
    pub(crate) kept: Vec<(String, Vec<u8>)>,
}

impl Attempt {
    pub(crate) fn new(run: u64, started_at: &'static str) -> Self {
        Self {
            workflow: ".github/workflows/test-health.yml",
            run,
            number: 1,
            started_at,
            test_runs: Vec::new(),
            jobs: Vec::new(),
            commit: "c0",
            head: None,
            branch: Some("main"),
            event: "schedule",
            kept: Vec::new(),
        }
    }
}

/// One test run of a fixture attempt, on Linux x64 in the Pre-merge suite of
/// the test-health tier, to be adjusted with [`with`].
pub(crate) fn test_run(test: &str, outcome: &str, tree: &str) -> Value {
    json!({
        "test": test,
        "runner": "nextest",
        "suite": "pre-merge",
        "tier": "test-health",
        "job": "quarantined",
        "os": "linux",
        "arch": "x64",
        "runtime": null,
        "shard": "1",
        "commit": format!("c{tree}"),
        "tree": tree,
        "outcome": outcome,
        "duration_seconds": 0.5,
        "order_seed": null,
        "entropy_seed": null,
    })
}

/// One job of a fixture attempt that ran `seconds` on runners with `labels`,
/// starting at `started_at`.
pub(crate) fn job(labels: &[&str], seconds: u64, started_at: &str) -> Value {
    json!({
        "job": "fixture",
        "labels": labels,
        "runner_os": "linux",
        "self_hosted": labels.contains(&"self-hosted"),
        "started_at": started_at,
        "completed_at": started_at,
        "duration_seconds": seconds,
        "conclusion": "success",
    })
}

/// Each cell of the Release suite's private-period matrix (ADR 0108), as
/// its JUnit artifact names its OS, architecture and runtime
/// (`junit-release-dispatch-<cell>-1`, `macos-x64-rosetta` for the
/// translated cell).
pub(crate) const RELEASE_CELLS: [(&str, &str, Option<&str>); 7] = [
    ("linux", "x64", Some("gnu")),
    ("linux", "x64", Some("musl")),
    ("linux", "arm64", Some("gnu")),
    ("linux", "arm64", Some("musl")),
    ("windows", "x64", None),
    ("macos", "arm64", None),
    ("macos", "x64", Some("rosetta")),
];

/// `run` once in each Release cell.
pub(crate) fn in_every_release_cell(run: &Value) -> Vec<Value> {
    RELEASE_CELLS
        .iter()
        .map(|(os, arch, runtime)| {
            let run = with(run.clone(), "os", json!(os));
            let run = with(run, "arch", json!(arch));
            with(run, "runtime", json!(runtime))
        })
        .collect()
}

/// `value` with `field` replaced.
pub(crate) fn with(mut value: Value, field: &str, replacement: Value) -> Value {
    value[field] = replacement;
    value
}

/// Writes each attempt as the store's compressed file for it.
pub(crate) fn write_store(workspace: &Workspace, attempts: &[Attempt]) {
    for attempt in attempts {
        let month = &attempt.started_at[..7].replace('-', "/");
        let name = format!(
            "{STORE}/runs/{month}/{}-{}.tar.gz",
            attempt.run, attempt.number
        );
        let run = json!({
            "repository": "goodfetch/goodfetch",
            "workflow": attempt.workflow,
            "run": attempt.run,
            "attempt": attempt.number,
            "event": attempt.event,
            "branch": attempt.branch,
            "commit": attempt.head.unwrap_or(attempt.commit),
            "tree": "0",
            "started_at": attempt.started_at,
        });
        let test_runs: Vec<Value> = attempt
            .test_runs
            .iter()
            .map(|test_run| {
                let mut test_run = test_run.clone();
                test_run["workflow"] = json!(attempt.workflow);
                test_run["run"] = json!(attempt.run);
                test_run["attempt"] = json!(attempt.number);
                test_run["started_at"] = json!(attempt.started_at);
                test_run
            })
            .collect();
        let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::default(),
        ));
        add(&mut archive, "run.json", run.to_string().as_bytes());
        add(&mut archive, "test-runs.jsonl", &lines(&test_runs));
        add(&mut archive, "jobs.jsonl", &lines(&attempt.jobs));
        let mut artifacts = std::collections::BTreeSet::new();
        for (path, contents) in &attempt.kept {
            add(&mut archive, &format!("junit/{path}"), contents);
            artifacts.extend(path.split_once('/').map(|(artifact, _)| artifact));
        }
        for artifact in artifacts {
            let context = json!({
                "commit": attempt.commit, "tree": "0", "job": "fixture",
                "order_seed": null, "entropy_seed": null,
            });
            add(
                &mut archive,
                &format!("junit/{artifact}/context.json"),
                context.to_string().as_bytes(),
            );
        }
        let bytes = archive.into_inner().unwrap().finish().unwrap();
        workspace.write(&name, &bytes);
    }
}

/// What one store file holds: the paths of its entries, then its run, test
/// runs and jobs as JSON.
pub(crate) struct StoreFile {
    pub(crate) paths: Vec<String>,
    pub(crate) run: Value,
    pub(crate) test_runs: Vec<Value>,
    pub(crate) jobs: Vec<Value>,
}

/// Reads the store file at `relative` in the workspace.
pub(crate) fn read_store_file(workspace: &Workspace, relative: &str) -> StoreFile {
    let bytes = workspace
        .read(relative)
        .unwrap_or_else(|| panic!("{relative} is missing"));
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes.as_slice()));
    let mut file = StoreFile {
        paths: Vec::new(),
        run: Value::Null,
        test_runs: Vec::new(),
        jobs: Vec::new(),
    };
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().into_owned();
        let mut text = String::new();
        std::io::Read::read_to_string(&mut entry, &mut text).unwrap();
        let parsed = || -> Vec<Value> {
            text.lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        };
        match path.as_str() {
            "run.json" => file.run = serde_json::from_str(&text).unwrap(),
            "test-runs.jsonl" => file.test_runs = parsed(),
            "jobs.jsonl" => file.jobs = parsed(),
            _ => {}
        }
        file.paths.push(path);
    }
    file.paths.sort();
    file
}

fn lines(values: &[Value]) -> Vec<u8> {
    let mut text = Vec::new();
    for value in values {
        writeln!(text, "{value}").unwrap();
    }
    text
}

fn add(archive: &mut tar::Builder<impl std::io::Write>, path: &str, contents: &[u8]) {
    let mut header = tar::Header::new_gnu();
    header.set_size(contents.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    archive.append_data(&mut header, path, contents).unwrap();
}
