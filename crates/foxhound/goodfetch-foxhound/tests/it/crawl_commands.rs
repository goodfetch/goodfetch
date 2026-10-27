//! Foxhound's crawl commands, generated from its catalog beside Collie's
//! (ADRs 0069, 0095 and 0099), through Foxhound's executable on the Daemon
//! of the test's home against the Proving ground: `crawl start` starts a
//! code-free crawl and prints its Run ID; `run outcomes --follow` prints
//! each ending as one JSON object per line until the Run ends; `run
//! records` prints the Records committed, one JSON array under `--json`,
//! once the call ends; `run failed-requests` and `run export` print one
//! JSON value of `goodfetch-protocol`'s type; `run add-seeds` reads its
//! seeds from stdin. Each `--json` result is validated against the
//! committed bundle.
#![expect(
    clippy::disallowed_methods,
    reason = "a test writes the settings file its commands read"
)]

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use goodfetch_test_support::{ProvingGround, TestEnv, cli};
use serde_json::{Value, json};

use crate::fetch_tiers::settings;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The Proving ground's seed.
const SEED: u64 = 1512;

/// The task whose index links Markdown pages.
const MARKDOWN_TASK: &str = "extraction/7";

/// A user configuration whose `daemon.idle_timeout` bounds a Daemon a
/// test forgets.
pub(crate) fn short_idle(env: &TestEnv) {
    env.user_configuration("daemon.idle_timeout = 60000\n");
}

/// Stops the test home's Daemon when the test ends.
pub(crate) struct StopsTheDaemon<'env>(pub(crate) &'env TestEnv);

impl Drop for StopsTheDaemon<'_> {
    fn drop(&mut self) {
        let _ = cli(self.0, FOXHOUND).args(["daemon", "stop"]).run();
    }
}

/// A settings file for a crawl of `proving_ground` over HTTP with no
/// delay, which `--settings` reads.
pub(crate) fn settings_file(env: &TestEnv, proving_ground: &ProvingGround) -> PathBuf {
    let path = env.folder().join(env.unique_name("settings") + ".json");
    let more = json!({
        "fetch": { "tier": "http" },
        "politeness": { "domain": { "delay": 0 } },
    });
    std::fs::write(&path, settings(proving_ground, &more).to_string()).unwrap();
    path
}

/// Starts the code-free crawl `run_id` of `seeds` with `follow: none`,
/// each page's Markdown in its Record, through `crawl start --json`, and
/// answers the Run ID it printed.
pub(crate) fn crawl_start(env: &TestEnv, settings: &Path, run_id: &str, seeds: &[String]) -> Value {
    let mut arguments = vec![
        "crawl".to_owned(),
        "start".to_owned(),
        "--run-id".to_owned(),
        run_id.to_owned(),
    ];
    for seed in seeds {
        arguments.push("--seeds".to_owned());
        arguments.push(json!({ "url": seed, "label": "page" }).to_string());
    }
    arguments.extend([
        "--extraction".to_owned(),
        json!({ "formats": { "markdown": true }, "follow": "none" }).to_string(),
        "--settings".to_owned(),
        settings.to_str().unwrap().to_owned(),
        "--json".to_owned(),
    ]);
    let started = cli(env, FOXHOUND)
        .args(&arguments)
        .json("RunStarted")
        .unwrap();
    assert_eq!(started.output.exit, Some(0), "{}", started.output);
    started.documents[0]["run_id"].clone()
}

/// A code-free crawl started with `crawl start`, followed with `run
/// outcomes --follow` until its Run ends, then read with `run records`,
/// `run failed-requests` and `run export`, each under `--json`: the
/// endings arrive one per line, the Records in one array, and every value
/// is the bundle's type.
///
/// Covers: story:11-1130, story:11-1131, command:crawl.start, command:run.outcomes, command:run.records, command:run.failed-requests, command:run.export
#[test]
fn a_code_free_crawl_started_followed_and_read_from_the_command_line() {
    let env = TestEnv::new();
    short_idle(&env);
    let _stops = StopsTheDaemon(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let settings = settings_file(&env, &proving_ground);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();

    let run_id = crawl_start(&env, &settings, "cli-crawl", std::slice::from_ref(&index));
    let outcomes = cli(&env, FOXHOUND)
        .args(["run", "outcomes", "cli-crawl", "--follow", "--json"])
        .json_lines("CrawlRequestOutcome")
        .unwrap();
    let records = cli(&env, FOXHOUND)
        .args([
            "run",
            "records",
            "cli-crawl",
            "--dataset",
            "pages",
            "--json",
        ])
        .json_array("RecordRead")
        .unwrap();
    let failed = cli(&env, FOXHOUND)
        .args(["run", "failed-requests", "cli-crawl", "--json"])
        .json("FailedRequests")
        .unwrap();
    let export = cli(&env, FOXHOUND)
        .args([
            "run",
            "export",
            "cli-crawl",
            "--dataset",
            "pages",
            "--format",
            "jsonl",
            "--json",
        ])
        .json("Export")
        .unwrap();

    let ended: Vec<(Value, Value)> = outcomes
        .documents
        .iter()
        .map(|outcome| (outcome["request"]["url"].clone(), outcome["kind"].clone()))
        .collect();
    let read: Vec<&Value> = records.documents[0].as_array().unwrap().iter().collect();
    let urls: BTreeSet<String> = read
        .iter()
        .map(|read| {
            read["record"]["url"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    assert_eq!(
        (
            run_id,
            ended,
            read.len(),
            urls,
            read.iter()
                .all(|read| read["record"]["value"]["markdown"].is_string()),
            failed.documents[0].clone(),
            export.documents[0]["records"].clone(),
            [
                outcomes.output.exit,
                records.output.exit,
                failed.output.exit,
                export.output.exit
            ],
        ),
        (
            json!("cli-crawl"),
            vec![(json!(index), json!("succeeded"))],
            1,
            BTreeSet::from([index.clone()]),
            true,
            json!({ "requests": [] }),
            json!(1),
            [Some(0); 4],
        ),
        "{}",
        outcomes.output
    );
}

/// The messages a command's Connection carried, from its `trace` records
/// on stderr: one line per direction, kind and method, with how many, in
/// the order each first came.
fn protocol_counts(stderr: &str) -> String {
    let mut counted: Vec<(String, usize)> = Vec::new();
    for line in stderr.lines() {
        let Some((_, record)) = line.split_once("goodfetch::cli::wire: message ") else {
            continue;
        };
        let field = |name: &str| {
            record
                .split(' ')
                .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
                .unwrap_or_default()
                .trim_matches('"')
                .to_owned()
        };
        let key = format!(
            "{} {} {}",
            field("direction"),
            field("kind"),
            field("method")
        );
        match counted.iter_mut().find(|(seen, _)| *seen == key) {
            Some((_, count)) => *count += 1,
            None => counted.push((key, 1)),
        }
    }
    counted
        .iter()
        .fold(String::new(), |mut text, (key, count)| {
            let _ = writeln!(text, "{count} x {}", key.trim_end());
            text
        })
}

/// Protocol counts (ADR 0071): the messages `crawl start`, `run outcomes
/// --follow` of the crawl it started, once that crawl ended, and `run
/// records` carry on their Connections to the Daemon, as snapshots.
///
/// Covers: story:11-1131, command:crawl.start, command:run.outcomes, command:run.records
#[test]
fn protocol_counts_of_crawl_start_run_outcomes_follow_and_run_records() {
    let env = TestEnv::new();
    short_idle(&env);
    let _stops = StopsTheDaemon(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let settings = settings_file(&env, &proving_ground);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    env.log_level("trace");

    let started = cli(&env, FOXHOUND)
        .args(["crawl", "start", "--run-id", "counted", "--seeds"])
        .arg(json!({ "url": index, "label": "page" }).to_string())
        .args([
            "--extraction",
            r#"{"formats":{"markdown":true},"follow":"none"}"#,
        ])
        .args(["--settings", settings.to_str().unwrap()])
        .run()
        .unwrap();
    // The crawl has ended once its last ending is read, so each counted
    // command reads it whole at once.
    let _ = cli(&env, FOXHOUND)
        .args(["run", "outcomes", "counted", "--follow", "--json"])
        .run()
        .unwrap();
    let followed = cli(&env, FOXHOUND)
        .args(["run", "outcomes", "counted", "--follow", "--json"])
        .run()
        .unwrap();
    let records = cli(&env, FOXHOUND)
        .args(["run", "records", "counted", "--dataset", "pages", "--json"])
        .run()
        .unwrap();

    insta::assert_snapshot!(
        "protocol_counts",
        format!(
            "crawl start\n{}\nrun outcomes --follow\n{}\nrun records\n{}",
            protocol_counts(&started.stderr),
            protocol_counts(&followed.stderr),
            protocol_counts(&records.stderr),
        )
    );
}

/// `run add-seeds` reads one seed per line from stdin, a URL or a seed as
/// JSON, and prints the engine's answer to each batch as one JSON object
/// per line under `--json`, into a running crawl that `crawl.keep_alive`
/// keeps open.
///
/// Covers: story:11-1130, command:run.add-seeds
#[test]
fn run_add_seeds_reads_its_seeds_from_stdin() {
    let env = TestEnv::new();
    short_idle(&env);
    let _stops = StopsTheDaemon(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let settings = settings_file(&env, &proving_ground);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    let path = env.folder().join("keep-alive.json");
    let mut kept: Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    kept["crawl"] = json!({ "keep_alive": true });
    std::fs::write(&path, kept.to_string()).unwrap();
    crawl_start(&env, &path, "seeded", std::slice::from_ref(&index));

    let added = cli(&env, FOXHOUND)
        .args(["run", "add-seeds", "seeded", "--json"])
        .stdin(format!(
            "{}\n\n{}\n",
            json!({ "url": format!("{index}#again"), "label": "page" }),
            json!({ "url": "not a url", "label": "page" }),
        ))
        .json_lines("EnqueuedBatch")
        .unwrap();
    let finished = cli(&env, FOXHOUND)
        .args(["run", "finish", "seeded", "--json"])
        .run()
        .unwrap();

    let answers: Vec<(bool, Value)> = added
        .documents
        .iter()
        .flat_map(|batch| batch["answers"].as_array().unwrap().clone())
        .map(|answer| (answer["added"] == true, answer["reason"].clone()))
        .collect();
    assert_eq!(
        (answers, added.output.exit, finished.exit),
        (
            vec![(false, json!("duplicate")), (false, json!("invalid_url")),],
            Some(0),
            Some(0),
        ),
        "{}",
        added.output
    );
}
