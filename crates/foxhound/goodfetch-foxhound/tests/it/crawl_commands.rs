//! Foxhound's crawl commands, generated from its catalog beside Collie's
//! (ADRs 0069, 0095 and 0099), through Foxhound's executable on the Daemon
//! of the test's home against the Proving ground: `crawl start` starts a
//! code-free crawl and prints its Run ID; `run outcomes --follow` prints
//! each ending as one JSON object per line until the Run ends; `run
//! records` prints the Records committed when it began as they arrive, one
//! JSON array under `--json`; `run failed-requests` and `run export` print
//! one JSON value of `goodfetch-protocol`'s type; `run add-seeds` reads its
//! seeds from stdin. Each `--json` result is validated against the
//! committed bundle, and each read of a crawl runs on an Attached engine
//! and on the Daemon, both against one snapshot of its output, human and
//! `--json` alike (ADR 0069).
#![expect(
    clippy::disallowed_methods,
    reason = "a test writes the settings file its commands read"
)]

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use goodfetch_test_support::record_command;
use goodfetch_test_support::{
    Cli, CliError, CliJson, DAEMON_DEADLINE, Daemon, Mode, ProvingGround, TestEnv,
    assert_cli_snapshot, cli,
};
use rstest::rstest;
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

/// Starts the Daemon of the test's home, which the test's commands find
/// rather than start their own, and makes the test fail on any process
/// left running from its home. A Daemon a command starts runs in a process
/// group of its own, out of the test's record, so a test killed on its
/// timeout left it running, and a keep-alive Run kept it from its idle stop
/// for hours (issue 1797). This one is the test's child: its drop stops it,
/// and a kill of the test's process group kills it too.
pub(crate) fn serve_the_home(env: &TestEnv) -> Daemon {
    env.fail_on_processes_from_home();
    Daemon::start_program(env, Path::new(FOXHOUND), DAEMON_DEADLINE).unwrap()
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

/// Starts the code-free crawl `run_id` of `seeds` with `--links none`,
/// each page's Markdown in its Record, through `crawl start --json`, and
/// answers the Run ID it printed, its one JSON value.
pub(crate) fn crawl_start(env: &TestEnv, settings: &Path, run_id: &str, seeds: &[String]) -> Value {
    let started = cli(env, FOXHOUND)
        .args(["crawl", "start"])
        .args(seeds)
        .args([
            "--run-id", run_id, "--format", "markdown", "--links", "none",
        ])
        .args(["--settings", settings.to_str().unwrap(), "--json"])
        .json("RunId")
        .unwrap();
    assert_eq!(started.output.exit, Some(0), "{}", started.output);
    started.documents[0].clone()
}

/// The flag that runs a read of a crawl in `mode`: the crawl's Daemon
/// runs, so `--daemon` reaches it, and `--attached` reads on an Attached
/// engine of the command's own.
fn engine_flag(mode: Mode) -> &'static str {
    match mode {
        Mode::Daemon => "--daemon",
        Mode::Attached => "--attached",
    }
}

/// Runs the read `arguments` of a crawl with `flag`, under `--json`,
/// where `check` validates it, then as human output, and snapshots both
/// outputs as `name`, a snapshot each mode's case shares. Beside the
/// shared filters, it masks the Proving ground's port, an Export's size
/// and each SHA-256 digest, since an Export holds its Records' commit
/// times, whose written length varies, and leaves out the dispatch records.
/// Answers the `--json` run.
fn read_snapshotted(
    env: &TestEnv,
    flag: &str,
    name: &str,
    arguments: &[&str],
    check: impl FnOnce(Cli<'_>) -> Result<CliJson, CliError>,
) -> CliJson {
    let checked = check(cli(env, FOXHOUND).args(arguments).args(["--json", flag]))
        .unwrap_or_else(|error| panic!("{arguments:?}: {error}"));
    let human = cli(env, FOXHOUND).args(arguments).arg(flag).run().unwrap();
    for (form, output) in [("json", checked.output.clone()), ("human", human)] {
        let output = output.without_dispatch_records();
        let mut settings = output.settings();
        settings.add_filter(r"\.test:[0-9]+\b", ".test:[PORT]");
        settings.add_filter(r"\b[0-9a-f]{64}\b", "[SHA256]");
        settings.add_filter(r#"\b(size"?: )[0-9]+"#, "${1}[SIZE]");
        settings.bind(|| assert_cli_snapshot!(format!("{name}_{form}"), output));
    }
    checked
}

/// A code-free crawl started with `crawl start`, followed with `run
/// outcomes --follow` until its Run ends, then read with `run records`,
/// `run failed-requests` and `run export`, each under `--json` and as
/// human output, on an Attached engine and on the Daemon alike: the
/// endings arrive one per line, the Records in one array, and every value
/// is the bundle's type.
///
/// Covers: story:11-1130, story:11-1131, command:crawl.start, command:run.outcomes, command:run.records, command:run.failed-requests, command:run.export
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn a_code_free_crawl_started_followed_and_read_from_the_command_line(#[case] mode: Mode) {
    let env = TestEnv::new();
    short_idle(&env);
    let _daemon = serve_the_home(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let settings = settings_file(&env, &proving_ground);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    let flag = engine_flag(mode);

    let run_id = crawl_start(&env, &settings, "cli-crawl", std::slice::from_ref(&index));
    let outcomes = read_snapshotted(
        &env,
        flag,
        "read_run_outcomes",
        &["run", "outcomes", "cli-crawl", "--follow"],
        |cli| cli.json_lines("CrawlRequestOutcome"),
    );
    let records = read_snapshotted(
        &env,
        flag,
        "read_run_records",
        &["run", "records", "cli-crawl", "--dataset", "pages"],
        |cli| cli.json_array("RecordRead"),
    );
    let failed = read_snapshotted(
        &env,
        flag,
        "read_run_failed_requests",
        &["run", "failed-requests", "cli-crawl"],
        |cli| cli.json("FailedRequests"),
    );
    let exported = env.folder().join("pages.jsonl");
    let export = read_snapshotted(
        &env,
        flag,
        "read_run_export",
        &[
            "run",
            "export",
            "cli-crawl",
            "pages",
            "--format",
            "jsonl",
            "--out",
            exported.to_str().unwrap(),
        ],
        |cli| cli.json("Export"),
    );

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

/// The other reads of a crawl that ended, and `run retry-failed`, each
/// under `--json` and as human output, on an Attached engine and on the
/// Daemon alike: its seeds, its rejected Records, its Datasets, a page of
/// one, an SQL query over it and a retry with nothing to retry, each one
/// JSON value of the bundle's type.
///
/// Covers: story:11-1131, command:run.seeds, command:run.rejected-records, command:run.retry-failed, command:dataset.list, command:dataset.page, command:dataset.query
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn a_crawls_other_commands_each_print_one_value_of_their_type(#[case] mode: Mode) {
    let env = TestEnv::new();
    short_idle(&env);
    let _daemon = serve_the_home(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let settings = settings_file(&env, &proving_ground);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    crawl_start(&env, &settings, "read", std::slice::from_ref(&index));
    // The crawl has ended once its last ending is read.
    let _ = cli(&env, FOXHOUND)
        .args(["run", "outcomes", "read", "--follow", "--json"])
        .run()
        .unwrap();

    let commands: [(&[&str], &str); 6] = [
        (&["run", "seeds", "read"], "RunSeeds"),
        (
            &["run", "rejected-records", "read", "--dataset", "pages"],
            "RejectedRecords",
        ),
        (&["run", "retry-failed", "read"], "RetriedFailed"),
        (&["dataset", "list", "--run-id", "read"], "DatasetList"),
        (
            &["dataset", "page", "--run-id", "read", "--dataset", "pages"],
            "DatasetPage",
        ),
        (
            &[
                "dataset",
                "query",
                "--runs",
                "read",
                "--sql",
                "SELECT count(*) AS records FROM records('read')",
            ],
            "DatasetQueryResult",
        ),
    ];
    let flag = engine_flag(mode);
    let printed: Vec<(String, Option<i32>, Value)> = commands
        .iter()
        .map(|(arguments, schema)| {
            let name = format!("read_{}", arguments[..2].join("_").replace('-', "_"));
            let checked = read_snapshotted(&env, flag, &name, arguments, |cli| cli.json(schema));
            (
                arguments[..2].join(" "),
                checked.output.exit,
                checked.documents[0].clone(),
            )
        })
        .collect();

    let exits: Vec<(String, Option<i32>)> = printed
        .iter()
        .map(|(command, exit, _)| (command.clone(), *exit))
        .collect();
    let expected: Vec<(String, Option<i32>)> = commands
        .iter()
        .map(|(arguments, _)| (arguments[..2].join(" "), Some(0)))
        .collect();
    assert_eq!(
        (exits, printed[2].2.clone()),
        (expected, json!({ "readmitted": 0 })),
        "{printed:?}"
    );
}

/// The messages a command's Connection carried, from its `trace` records
/// on stderr: one line per direction, kind and method, with how many and
/// their bytes after the shared redactions, which the CLI measures, in the
/// order each first came.
fn protocol_counts(stderr: &str) -> String {
    let mut counted: Vec<(String, usize, usize)> = Vec::new();
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
        let bytes: usize = field("bytes")
            .parse()
            .expect("a wire record names its bytes");
        match counted.iter_mut().find(|(seen, ..)| *seen == key) {
            Some((_, count, total)) => {
                *count += 1;
                *total += bytes;
            }
            None => counted.push((key, 1, bytes)),
        }
    }
    counted
        .iter()
        .fold(String::new(), |mut text, (key, count, bytes)| {
            let _ = writeln!(text, "{count} x {}, {bytes} bytes", key.trim_end());
            text
        })
}

/// The protocol counts of a followed crawl: the progress reads, each
/// answering a change, and so the answers, vary with how fast the crawl
/// goes, so their counts read `N` and their bytes are left out.
fn followed_counts(stderr: &str) -> String {
    protocol_counts(stderr)
        .lines()
        .map(|line| {
            let (count, key) = line
                .split_once(" x ")
                .expect("a protocol count reads `<count> x <key>`");
            match key.rsplit_once(", ") {
                Some((key, _)) if key == "received response" || key.ends_with(" run.progress") => {
                    format!("N x {key}\n")
                }
                _ => format!("{count} x {key}\n"),
            }
        })
        .collect()
}

/// Protocol counts (ADR 0071): the messages `crawl start --follow`, `run
/// outcomes --follow` of the crawl it started, once that crawl ended, and
/// `run records` carry on their Connections to the Daemon, as snapshots.
///
/// Covers: story:11-1131, story:11-1132, command:crawl.start, command:run.outcomes, command:run.records
#[test]
fn protocol_counts_of_crawl_start_run_outcomes_follow_and_run_records() {
    let env = TestEnv::new();
    short_idle(&env);
    let _daemon = serve_the_home(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let settings = settings_file(&env, &proving_ground);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    env.log_level("trace");

    let started = cli(&env, FOXHOUND)
        .args(["crawl", "start", &index, "--run-id", "counted"])
        .args(["--links", "none", "--follow"])
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
            "crawl start --follow\n{}\nrun outcomes --follow\n{}\nrun records\n{}",
            followed_counts(&started.stderr),
            protocol_counts(&followed.stderr),
            protocol_counts(&records.stderr),
        )
    );
}

/// A settings file as [`settings_file`]'s, whose crawl `crawl.keep_alive`
/// keeps running once its Frontier is empty, until `run.finish`.
fn keep_alive_settings(env: &TestEnv, proving_ground: &ProvingGround) -> PathBuf {
    let settings = settings_file(env, proving_ground);
    let mut kept: Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    kept["crawl"] = json!({ "keep_alive": true });
    std::fs::write(&settings, kept.to_string()).unwrap();
    settings
}

/// The progress of the Run `run_id` once `reached` holds of it, read with
/// `run progress --after`, which answers each change as the engine signals
/// it.
fn progress_once(env: &TestEnv, run_id: &str, reached: impl Fn(&Value) -> bool) -> Value {
    let mut after: Option<String> = None;
    loop {
        let mut command = cli(env, FOXHOUND).args(["run", "progress", run_id, "--json"]);
        if let Some(after) = &after {
            command = command.args(["--after", after]);
        }
        let read = command.json("RunProgress").unwrap();
        let progress = read.documents[0].clone();
        assert_eq!(read.output.exit, Some(0), "{}", read.output);
        if reached(&progress) {
            return progress;
        }
        assert_ne!(progress["last"], true, "the Run ended first: {progress}");
        after = Some(progress["position"].to_string());
    }
}

/// `run records` without `--follow` on a live keep-alive Run prints the
/// Records committed when it began, one JSON array, and ends while the Run
/// still runs, in either mode, rather than wait for an end that only
/// `run.finish` brings.
///
/// Covers: story:11-1131, command:run.records
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn run_records_on_a_live_run_prints_what_was_committed_and_ends(#[case] mode: Mode) {
    let env = TestEnv::new();
    short_idle(&env);
    let _daemon = serve_the_home(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let settings = keep_alive_settings(&env, &proving_ground);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    // On the Daemon, which the crawl outlives the command on.
    crawl_start(&env, &settings, "live", std::slice::from_ref(&index));
    progress_once(&env, "live", |progress| progress["idle"] == true);
    let flag = engine_flag(mode);

    let records = cli(&env, FOXHOUND)
        .args([
            "run",
            "records",
            "live",
            "--dataset",
            "pages",
            "--json",
            flag,
        ])
        .json_array("RecordRead")
        .unwrap();
    let still = cli(&env, FOXHOUND)
        .args(["run", "progress", "live", "--json"])
        .json("RunProgress")
        .unwrap();
    let finished = cli(&env, FOXHOUND)
        .args(["run", "finish", "live", "--json"])
        .run()
        .unwrap();

    let urls: Vec<Value> = records.documents[0]
        .as_array()
        .unwrap()
        .iter()
        .map(|read| read["record"]["url"].clone())
        .collect();
    assert_eq!(
        (
            records.output.exit,
            urls,
            still.documents[0]["lifecycle"].clone(),
            finished.exit
        ),
        (Some(0), vec![json!(index)], json!("running"), Some(0)),
        "{}",
        records.output
    );
}

/// `crawl start --detached` of a code-free crawl on the Daemon prints its
/// Run ID, and the Run, no Connection's, goes on after the command's
/// Connection ends, until `run finish` (ADRs 0078 and 0102).
///
/// Covers: story:3-174, command:crawl.start
#[test]
fn a_code_free_crawl_started_detached_goes_on_after_its_command_ends() {
    let env = TestEnv::new();
    short_idle(&env);
    let _daemon = serve_the_home(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let settings = keep_alive_settings(&env, &proving_ground);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();

    let started = cli(&env, FOXHOUND)
        .args(["crawl", "start", &index])
        .args([
            "--run-id", "kept", "--format", "markdown", "--links", "none",
        ])
        .args([
            "--settings",
            settings.to_str().unwrap(),
            "--detached",
            "--json",
        ])
        .json("RunId")
        .unwrap();
    let progress = progress_once(&env, "kept", |progress| progress["idle"] == true);
    let finished = cli(&env, FOXHOUND)
        .args(["run", "finish", "kept", "--json"])
        .run()
        .unwrap();

    assert_eq!(
        (
            started.output.exit,
            started.documents[0].clone(),
            progress["lifecycle"].clone(),
            finished.exit
        ),
        (Some(0), json!("kept"), json!("running"), Some(0)),
        "{}",
        started.output
    );
}

/// A seeds file whose seeds pass the engine's message limit (16 MiB) in
/// one `crawl.start` starts the crawl, prints its Run ID, and adds every
/// seed, the start's and `run.add_seeds`' alike; here each a long URL of
/// the Proving ground's that a Politeness delay keeps from being fetched.
///
/// Covers: story:11-1132, command:crawl.start
#[test]
fn a_seeds_file_past_the_message_limit_starts_the_crawl_and_adds_every_seed() {
    const SEEDS: usize = 9_000;
    let env = TestEnv::new();
    short_idle(&env);
    let _daemon = serve_the_home(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let slow_settings = env.folder().join("slow.json");
    let slow = json!({
        "fetch": { "tier": "http" },
        "politeness": { "domain": { "delay": 600_000 } },
    });
    std::fs::write(&slow_settings, settings(&proving_ground, &slow).to_string()).unwrap();
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    // Seeds of about 2,000 bytes, under `crawl.max_url_length`, 18 MB in all.
    let long = "q".repeat(1_990);
    let lines: Vec<String> = (0..SEEDS)
        .map(|n| format!("{index}?n={n}&{long}"))
        .collect();
    let file = env.folder().join("seeds.txt");
    std::fs::write(&file, lines.join("\n")).unwrap();

    let started = cli(&env, FOXHOUND)
        .args(["crawl", "start", "--run-id", "many", "--links", "none"])
        .args(["--seeds-file", file.to_str().unwrap()])
        .args(["--settings", slow_settings.to_str().unwrap(), "--json"])
        .json("RunId")
        .unwrap();
    let progress = cli(&env, FOXHOUND)
        .args(["run", "progress", "many", "--json"])
        .json("RunProgress")
        .unwrap();
    let stopped = cli(&env, FOXHOUND)
        .args(["run", "abort", "many"])
        .run()
        .unwrap();

    let requests = &progress.documents[0]["requests"];
    let admitted: u64 = [
        "pending",
        "fetching",
        "handling",
        "in_human_pause",
        "succeeded",
        "failed",
        "skipped",
        "blocked",
    ]
    .iter()
    .map(|state| requests[*state].as_u64().unwrap_or_default())
    .sum();
    assert_eq!(
        (started.output.exit, started.documents[0].clone(), admitted,),
        (Some(0), json!("many"), SEEDS as u64),
        "{}\n{stopped}",
        started.output
    );
}

/// `run add-seeds` sends each line of stdin as it comes, so its answer
/// prints while stdin stays open, and an interrupt ends it at once,
/// stdin open and idle, exiting `130`.
///
/// Covers: story:11-1130, command:run.add-seeds, exit:interrupted
#[cfg(unix)]
#[test]
fn run_add_seeds_sends_each_line_as_it_comes_and_ends_at_an_interrupt() {
    use std::io::{BufRead as _, BufReader, Write as _};

    const ARGUMENTS: [&str; 4] = ["run", "add-seeds", "fed", "--json"];
    let env = TestEnv::new();
    short_idle(&env);
    let _daemon = serve_the_home(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let settings = keep_alive_settings(&env, &proving_ground);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    crawl_start(&env, &settings, "fed", std::slice::from_ref(&index));
    let mut command = env.process_command(FOXHOUND);
    command.args(ARGUMENTS).stderr(std::process::Stdio::null());
    let process = env.spawn(&mut command).unwrap();
    let mut stdin = process.child().stdin.take().unwrap();
    writeln!(
        stdin,
        "{}",
        json!({ "url": format!("{index}#again"), "label": "page" })
    )
    .unwrap();
    stdin.flush().unwrap();
    let mut stdout = BufReader::new(process.child().stdout.take().unwrap());
    let mut answered = String::new();
    stdout.read_line(&mut answered).unwrap();

    process.interrupt().unwrap();
    let status = process.child().wait().unwrap();
    record_command(&env, &ARGUMENTS, status.code()).unwrap();
    drop(stdin);
    let finished = cli(&env, FOXHOUND)
        .args(["run", "finish", "fed"])
        .run()
        .unwrap();

    let answered: Value = serde_json::from_str(&answered).unwrap();
    assert_eq!(
        (
            answered["answers"][0]["reason"].clone(),
            status.code(),
            finished.exit
        ),
        (json!("duplicate"), Some(130), Some(0)),
        "{answered}"
    );
}

/// `run add-seeds` refuses a line of stdin that is not UTF-8 as a usage
/// error naming its line, exit `2`, once it added the seeds before it.
///
/// Covers: story:11-1130, command:run.add-seeds, exit:usage
#[test]
fn run_add_seeds_refuses_a_line_that_is_not_utf8_naming_it() {
    let env = TestEnv::new();
    short_idle(&env);
    let _daemon = serve_the_home(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let settings = keep_alive_settings(&env, &proving_ground);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    crawl_start(&env, &settings, "refused", std::slice::from_ref(&index));
    let mut input = json!({ "url": format!("{index}#first"), "label": "page" })
        .to_string()
        .into_bytes();
    input.extend(b"\n\xff\xfe\n");
    input.extend(
        json!({ "url": format!("{index}#third"), "label": "page" })
            .to_string()
            .into_bytes(),
    );

    let added = cli(&env, FOXHOUND)
        .args(["run", "add-seeds", "refused", "--json"])
        .stdin(input)
        .run()
        .unwrap();
    let finished = cli(&env, FOXHOUND)
        .args(["run", "finish", "refused"])
        .run()
        .unwrap();

    assert_eq!(
        (
            added.exit,
            added
                .stderr
                .contains("line 2 of standard input is not UTF-8"),
            added.stdout.matches(r#""reason":"duplicate""#).count(),
            finished.exit,
        ),
        (Some(2), true, 1, Some(0)),
        "{added}"
    );
}

/// `run add-seeds` reads one seed per line from stdin, a URL or a seed as
/// JSON, and prints the engine's answer to each batch as one JSON object
/// per line under `--json`, into a running crawl that `crawl.keep_alive`
/// keeps open; `run add-seed` adds one seed and prints its answer.
///
/// Covers: story:11-1130, command:run.add-seeds, command:run.add-seed
#[test]
fn run_add_seeds_reads_its_seeds_from_stdin() {
    let env = TestEnv::new();
    short_idle(&env);
    let _daemon = serve_the_home(&env);
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
    let one = cli(&env, FOXHOUND)
        .args(["run", "add-seed", "seeded", "--seed"])
        .arg(json!({ "url": index, "label": "page" }).to_string())
        .arg("--json")
        .json("Enqueued")
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
        (
            answers,
            one.documents[0]["reason"].clone(),
            [added.output.exit, one.output.exit, finished.exit],
        ),
        (
            vec![(false, json!("duplicate")), (false, json!("invalid_url"))],
            json!("duplicate"),
            [Some(0); 3],
        ),
        "{}",
        added.output
    );
}
