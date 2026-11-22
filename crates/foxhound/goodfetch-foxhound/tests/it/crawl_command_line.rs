//! A crawl run from the command line alone (ADRs 0069, 0086, 0099 and
//! 0116; spec #611's CLI), through Foxhound's executable: `crawl start
//! <url>…` builds the code-free crawl from its flags and prints its Run ID,
//! under `--follow` exits by how the Run ended, checkpoints it at a first
//! interrupt, holds it to its `--limit`, and prints a failure once the Run
//! ID is out on stderr alone;
//! `run export --out` writes the Export's file whole or not at all, and
//! stops at a standard output that takes no more; and `config explain
//! politeness` shows every Politeness value with its layer before a crawl
//! starts. The Scripted engine plays the Daemon whose Connection fails.
#![expect(
    clippy::disallowed_methods,
    reason = "a test writes the files its commands read and reads the files they write"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io::{BufRead as _, BufReader};
use std::process::Stdio;

use goodfetch_test_support::ToolStatus;
use goodfetch_test_support::{
    Mode, ProvingGround, SnapshotValues, TestEnv, Tool, cli, engine_script, record_command,
    reported_address, snapshot_settings, tools,
};
use rstest::rstest;
use serde_json::{Value, json};

use crate::crawl_commands::{StopsTheDaemon, crawl_start, settings_file, short_idle};

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The Proving ground's seed.
const SEED: u64 = 1513;

/// The catalog family's first listing page, which links every product.
const CATALOG: &str = "catalog/1";

/// The task whose index links Markdown pages.
const MARKDOWN_TASK: &str = "extraction/7";

/// The size of the filesystem a full-filesystem case fills.
#[cfg(target_os = "linux")]
const SIZE: u64 = 64 * 1024;

/// `crawl start` with `--format`, `--field`, `--fields`, `--links` and a
/// seeds file crawls the catalog family and, under `--follow --json`,
/// prints the Run ID alone and exits `0` once the Run finished; the
/// Records it committed hold every product the oracle knows.
///
/// Covers: story:11-1132, command:crawl.start, command:run.records
#[test]
fn crawl_start_crawls_the_catalog_from_its_flags_and_prints_the_run_id() {
    let env = TestEnv::new();
    short_idle(&env);
    let _stops = StopsTheDaemon(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let settings = settings_file(&env, &proving_ground);
    let seeds = env.folder().join("seeds.txt");
    std::fs::write(&seeds, proving_ground.start_url(CATALOG).unwrap() + "\n").unwrap();
    let fields = env.folder().join("fields.json");
    std::fs::write(
        &fields,
        json!({
            "price": {
                "locator": { "kind": "css", "selector": "article.product .price" },
                "attribute": "data-price",
            },
        })
        .to_string(),
    )
    .unwrap();

    let started = cli(&env, FOXHOUND)
        .args(["crawl", "start", "--run-id", "catalog"])
        .args(["--seeds-file", seeds.to_str().unwrap()])
        .args(["--format", "markdown", "--links", "scope"])
        .args(["--field", "id=article.product .id"])
        .args(["--field", "name=article.product .name"])
        .args(["--fields", fields.to_str().unwrap()])
        .args(["--settings", settings.to_str().unwrap()])
        .args(["--follow", "--json"])
        .json("RunId")
        .unwrap();
    let records = cli(&env, FOXHOUND)
        .args(["run", "records", "catalog", "--dataset", "pages", "--json"])
        .json_array("RecordRead")
        .unwrap();

    let products: Vec<Value> = records.documents[0]
        .as_array()
        .unwrap()
        .iter()
        .map(|read| &read["record"]["value"])
        .filter(|value| value["id"].is_string())
        .map(|value| json!({ "id": value["id"], "name": value["name"], "price": value["price"] }))
        .collect();
    let judged = proving_ground
        .judge_records(CATALOG, ToolStatus::Success, &products)
        .unwrap();
    assert_eq!(
        (
            started.documents.clone(),
            started.output.exit,
            judged["outcome"].clone()
        ),
        (vec![json!("catalog")], Some(0), json!("verified")),
        "{}\n{judged}",
        started.output
    );
}

/// A keep-alive crawl, which waits for new work until `run.finish` lets
/// it end, started with `--follow` in the background.
fn followed_keep_alive_crawl(
    env: &TestEnv,
    proving_ground: &ProvingGround,
    run_id: &str,
    more: &[&str],
) -> (goodfetch_test_support::Process, Vec<String>) {
    let settings = settings_file(env, proving_ground);
    let mut kept: Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    kept["crawl"] = json!({ "keep_alive": true });
    std::fs::write(&settings, kept.to_string()).unwrap();
    let mut arguments: Vec<String> = [
        "crawl",
        "start",
        &proving_ground.start_url(MARKDOWN_TASK).unwrap(),
        "--run-id",
        run_id,
        "--links",
        "none",
        "--settings",
        settings.to_str().unwrap(),
        "--follow",
        "--json",
    ]
    .iter()
    .map(|argument| (*argument).to_owned())
    .collect();
    arguments.extend(more.iter().map(|argument| (*argument).to_owned()));
    let mut command = env.process_command(FOXHOUND);
    command
        .args(&arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let process = env.spawn(&mut command).unwrap();
    (process, arguments)
}

/// The Run ID a followed `crawl start` printed first, once its Run started.
fn printed_run_id(process: &goodfetch_test_support::Process) -> Value {
    let mut stdout = BufReader::new(process.child().stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

/// A followed crawl exits by how its Run ended: `20` when another command
/// stopped it, `0` when another let it finish `verified`, with or without
/// `--require-verified`, and `21` under `--require-verified` when it
/// finished with another Verdict: with `fetch.block_classification` off,
/// its Verdict is `needs_review` (ADR 0029).
///
/// Covers: story:11-1132, command:crawl.start
#[rstest]
/// Covers: command:run.stop, exit:run_stopped
#[case::stopped(&[], &["run", "stop"], Some(20))]
/// Covers: command:run.finish, exit:success
#[case::finished_verified(&["--require-verified"], &["run", "finish"], Some(0))]
/// Covers: exit:verdict_not_verified
#[case::finished_needing_review(
    &["--require-verified", "--set", "fetch.block_classification=false"],
    &["run", "finish"],
    Some(21)
)]
fn a_followed_crawl_exits_by_how_its_run_ended(
    #[case] more: &[&str],
    #[case] ended_by: &[&str],
    #[case] exit: Option<i32>,
) {
    let env = TestEnv::new();
    short_idle(&env);
    let _stops = StopsTheDaemon(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (process, arguments) = followed_keep_alive_crawl(&env, &proving_ground, "followed", more);
    let run_id = printed_run_id(&process);

    let ended = cli(&env, FOXHOUND)
        .args(ended_by)
        .arg("followed")
        .run()
        .unwrap();
    let status = process.child().wait().unwrap();
    record_command(&env, &arguments, status.code()).unwrap();

    assert_eq!(
        (run_id, status.code()),
        (json!("followed"), exit),
        "{ended}"
    );
}

/// `crawl start --limit` holds the crawl to its Limit: the followed crawl
/// stops once the Limit is used up and exits `20`, and a start that names
/// the Run again while the Limit is still used up fails `limit_reached`,
/// exit `16`, naming the Run's own Limit.
///
/// Covers: story:11-1132, command:crawl.start, exit:run_stopped, exit:limit_reached
#[test]
fn crawl_start_with_a_limit_stops_the_crawl_and_a_restart_exits_16() {
    let env = TestEnv::new();
    short_idle(&env);
    let _stops = StopsTheDaemon(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let settings = settings_file(&env, &proving_ground);
    let start = |more: &[&str]| {
        cli(&env, FOXHOUND)
            .args([
                "crawl",
                "start",
                &proving_ground.start_url(CATALOG).unwrap(),
            ])
            .args([
                "--run-id",
                "capped",
                "--links",
                "scope",
                "--limit",
                "requests=1",
            ])
            .args(["--settings", settings.to_str().unwrap(), "--json"])
            .args(more)
            .run()
            .unwrap()
    };

    let followed = start(&["--follow"]);
    let restarted = start(&[]);
    let refused: Value = serde_json::from_str(&restarted.stdout).unwrap_or(Value::Null);

    assert_eq!(
        (
            followed.exit,
            restarted.exit,
            refused["error"]["code"].clone(),
            refused["error"]["scope"].clone(),
        ),
        (
            Some(20),
            Some(16),
            json!("limit_reached"),
            json!({ "kind": "run", "id": "capped" }),
        ),
        "{followed}\n{restarted}"
    );
}

/// A first interrupt of a followed crawl stops its Run with a Checkpoint,
/// resumable by its Run ID, and exits `130` once it stopped.
///
/// Covers: story:11-1132, command:crawl.start, command:run.report, exit:interrupted
#[cfg(unix)]
#[test]
fn an_interrupt_checkpoints_a_followed_crawl_and_exits_130() {
    let env = TestEnv::new();
    short_idle(&env);
    let _stops = StopsTheDaemon(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (process, arguments) = followed_keep_alive_crawl(&env, &proving_ground, "followed", &[]);
    let _ = printed_run_id(&process);

    process.interrupt().unwrap();
    let status = process.child().wait().unwrap();
    record_command(&env, &arguments, status.code()).unwrap();
    let report = cli(&env, FOXHOUND)
        .args(["run", "report", "followed", "--json"])
        .json("RunReport")
        .unwrap();

    assert_eq!(
        (status.code(), report.documents[0]["lifecycle"].clone()),
        (Some(130), json!("stopped")),
        "{}",
        report.output
    );
}

/// A second interrupt of a followed crawl, sent once the first one's
/// `run.stop` is in flight, ends the command at once, exit `130`, while
/// that stop is still unanswered (ADR 0069's #1180 note). The Scripted
/// engine plays the Daemon that holds `run.stop` open: a real one's
/// `run.stop` abandons the attempts in flight and answers within
/// milliseconds, so no page holds it open, while the first interrupt's
/// test shows that it stops the Run with its Checkpoint.
///
/// Covers: story:11-1132, command:crawl.start, exit:interrupted
#[cfg(unix)]
#[test]
fn a_second_interrupt_ends_the_command_while_the_first_ones_stop_waits() {
    const ARGUMENTS: [&str; 8] = [
        "crawl",
        "start",
        "https://example.com/",
        "--run-id",
        "held",
        "--follow",
        "--json",
        "--daemon",
    ];
    let env = TestEnv::new();
    // The command's wire records show when it sent `run.stop`.
    env.log_level("info,goodfetch::cli::wire=trace");
    let scripted = scripted_daemon(&env, "daemon/cli-crawl-follow-stop-held.json");
    let mut command = env.process_command(FOXHOUND);
    command.args(ARGUMENTS).stderr(Stdio::piped());
    let process = env.spawn(&mut command).unwrap();
    let _ = printed_run_id(&process);

    process.interrupt().unwrap();
    let mut stderr = BufReader::new(process.child().stderr.take().unwrap());
    let mut line = String::new();
    while !(line.contains("goodfetch::cli::wire")
        && line.contains(r#"direction="sent""#)
        && line.contains(r#"method="run.stop""#))
    {
        line.clear();
        assert_ne!(
            stderr.read_line(&mut line).unwrap(),
            0,
            "the command never sent run.stop"
        );
    }
    let status = process.interrupt_and_wait(SECOND_INTERRUPT_DEADLINE);
    let code = status
        .as_ref()
        .ok()
        .and_then(std::process::ExitStatus::code);
    record_command(&env, &ARGUMENTS, code).unwrap();
    let played = scripted.child().wait().unwrap();

    assert_eq!((code, played.success()), (Some(130), true), "{status:?}");
}

/// How long a command may take to exit after a second interrupt: far
/// less than it would wait for the unanswered `run.stop`, which is for
/// ever.
#[cfg(unix)]
const SECOND_INTERRUPT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// Without `--follow`, `run records` prints each Record as it arrives,
/// before its call ends, so it keeps none of them; an interrupt then
/// cancels the call and closes the JSON array, so stdout still holds one
/// JSON value, and exits `130`.
///
/// Covers: story:11-1131, command:run.records, exit:interrupted
#[cfg(unix)]
#[test]
fn run_records_prints_each_record_as_it_arrives() {
    use std::io::Read as _;

    const ARGUMENTS: [&str; 7] = [
        "run",
        "records",
        "first",
        "--dataset",
        "pages",
        "--json",
        "--daemon",
    ];
    let env = TestEnv::new();
    let scripted = scripted_daemon(&env, "daemon/cli-run-records-streamed.json");
    let mut command = env.process_command(FOXHOUND);
    command.args(ARGUMENTS).stderr(Stdio::null());
    let process = env.spawn(&mut command).unwrap();
    let mut stdout = process.child().stdout.take().unwrap();
    let mut printed = Vec::new();
    let mut chunk = [0_u8; 4096];
    while !String::from_utf8_lossy(&printed).contains(r#""number":1"#) {
        let read = stdout.read(&mut chunk).unwrap();
        assert_ne!(read, 0, "the command ended before it printed the Record");
        printed.extend(&chunk[..read]);
    }

    process.interrupt().unwrap();
    let status = process.child().wait().unwrap();
    record_command(&env, &ARGUMENTS, status.code()).unwrap();
    stdout.read_to_end(&mut printed).unwrap();
    let played = scripted.child().wait().unwrap();

    let array: Value = serde_json::from_slice(&printed).unwrap_or(Value::Null);
    assert_eq!(
        (
            status.code(),
            array.as_array().map(Vec::len),
            array[0]["record"]["number"].clone(),
            played.success(),
        ),
        (Some(130), Some(1), json!(1), true),
        "{}",
        String::from_utf8_lossy(&printed)
    );
}

/// `run records --through` reads through the number given: the command
/// calls `run.records` with it at once, without reading the last committed
/// number first, and ends once the call answers.
///
/// Covers: story:11-1131, command:run.records
#[test]
fn run_records_given_through_reads_through_it_alone() {
    let env = TestEnv::new();
    let scripted = scripted_daemon(&env, "daemon/cli-run-records-through.json");

    let read = cli(&env, FOXHOUND)
        .args(["run", "records", "first", "--dataset", "pages"])
        .args(["--through", "1", "--json", "--daemon"])
        .json_array("RecordRead")
        .unwrap();
    let played = scripted.child().wait().unwrap();

    assert_eq!(
        (
            read.output.exit,
            read.documents[0].as_array().map(Vec::len),
            played.success(),
        ),
        (Some(0), Some(1), true),
        "{}",
        read.output
    );
}

/// An error whose id is null, the engine's answer to a message it could
/// not read, such as a `crawl.start` past its message limit, ends the
/// command with that error, `message_too_large`, exit `1`, rather than
/// leave it waiting for an answer that never comes.
///
/// Covers: story:11-1132, command:crawl.start, exit:internal
#[test]
fn an_error_whose_id_is_null_ends_the_command_with_it() {
    let env = TestEnv::new();
    let scripted = scripted_daemon(&env, "daemon/cli-crawl-start-unreadable.json");

    let output = cli(&env, FOXHOUND)
        .args([
            "crawl",
            "start",
            "https://example.com/",
            "--run-id",
            "first",
        ])
        .args(["--json", "--daemon"])
        .run()
        .unwrap();
    let played = scripted.child().wait().unwrap();
    let failed: Value = serde_json::from_str(&output.stdout).unwrap_or(Value::Null);

    assert_eq!(
        (
            output.exit,
            failed["error"]["code"].clone(),
            failed["error"]["operation"].clone(),
            played.success(),
        ),
        (
            Some(1),
            json!("message_too_large"),
            json!("crawl.start"),
            true
        ),
        "{output}"
    );
}

/// `run export --out` writes the Export's file at the path, whole, with
/// the bytes `--out -` streams to standard output, and leaves no other
/// file beside it.
///
/// Covers: story:11-1132, command:run.export
#[test]
fn run_export_writes_its_file_at_the_path_or_on_standard_output() {
    let env = TestEnv::new();
    short_idle(&env);
    let _stops = StopsTheDaemon(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let settings = settings_file(&env, &proving_ground);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    crawl_start(&env, &settings, "exported", std::slice::from_ref(&index));
    // The crawl has ended once its last ending is read.
    let _ = cli(&env, FOXHOUND)
        .args(["run", "outcomes", "exported", "--follow", "--json"])
        .run()
        .unwrap();
    let folder = env.folder().join("exports");

    let written = cli(&env, FOXHOUND)
        .args(["run", "export", "exported", "pages", "--format", "jsonl"])
        .args([
            "--out",
            folder.join("pages.jsonl").to_str().unwrap(),
            "--json",
        ])
        .json("Export")
        .unwrap();
    let streamed = cli(&env, FOXHOUND)
        .args(["run", "export", "exported", "pages", "--format", "jsonl"])
        .args(["--out", "-"])
        .run()
        .unwrap();

    let file = std::fs::read_to_string(folder.join("pages.jsonl")).unwrap();
    let names: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        (
            written.output.exit,
            streamed.exit,
            file.lines().count(),
            written.documents[0]["size"].clone(),
            file == streamed.stdout,
            names,
        ),
        (
            Some(0),
            Some(0),
            1,
            json!(file.len()),
            true,
            vec!["pages.jsonl".to_owned()]
        ),
        "{streamed}"
    );
}

/// `run export --out` to a path that cannot hold the file exits `13`,
/// `path_not_writable`, and to a full filesystem `14`,
/// `file_write_failed`, each leaving the path as it was.
///
/// Covers: story:11-1132, command:run.export
#[cfg(target_os = "linux")]
#[rstest]
/// Covers: exit:path_not_writable
#[case::path_through_a_file(false, Some(13), "path_not_writable")]
/// Covers: exit:file_write_failed
#[case::full_filesystem(true, Some(14), "file_write_failed")]
fn run_export_refuses_a_path_it_cannot_write_whole(
    #[case] full: bool,
    #[case] exit: Option<i32>,
    #[case] code: &str,
) {
    let env = TestEnv::new();
    short_idle(&env);
    let _stops = StopsTheDaemon(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let settings = settings_file(&env, &proving_ground);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    crawl_start(&env, &settings, "refused", std::slice::from_ref(&index));
    let _ = cli(&env, FOXHOUND)
        .args(["run", "outcomes", "refused", "--follow", "--json"])
        .run()
        .unwrap();
    let folder = env.folder().join("out");
    std::fs::create_dir_all(&folder).unwrap();
    // A file where the path's folder should be, or a filesystem whose
    // every page a file already holds.
    let (target, filler) = if full {
        (folder.join("pages.jsonl"), Some(folder.join("filler")))
    } else {
        std::fs::write(folder.join("file"), "a file").unwrap();
        (folder.join("file").join("pages.jsonl"), None)
    };
    if let Some(filler) = &filler {
        std::fs::write(filler, vec![0_u8; usize::try_from(SIZE).unwrap()]).unwrap();
    }
    let arguments = [
        "run",
        "export",
        "refused",
        "pages",
        "--format",
        "jsonl",
        "--out",
        target.to_str().unwrap(),
        "--json",
    ];

    let output = if full {
        let output = goodfetch_test_support::tmpfs_mounted(&env, &folder, SIZE, FOXHOUND)
            .args(arguments)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        record_command(&env, &arguments, output.status.code()).unwrap();
        (
            output.status.code(),
            String::from_utf8(output.stdout).unwrap(),
        )
    } else {
        let output = cli(&env, FOXHOUND).args(arguments).run().unwrap();
        (output.exit, output.stdout)
    };

    let failed: Value = serde_json::from_str(&output.1).unwrap();
    let left: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("pages"))
        .collect();
    assert_eq!(
        (output.0, failed["error"]["code"].clone(), left),
        (exit, json!(code), Vec::<String>::new()),
        "{failed}"
    );
}

/// `config explain politeness` lists every Politeness value with the
/// layer that set it, the built-in default, user configuration or a flag,
/// before any crawl starts, in either mode.
///
/// Covers: story:11-1132, command:config.explain
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn config_explain_politeness_lists_every_politeness_value_with_its_layer(#[case] mode: Mode) {
    let env = TestEnv::new();
    short_idle(&env);
    env.user_configuration("daemon.idle_timeout = 60000\npoliteness.domain.delay = 2000\n");
    let _stops = StopsTheDaemon(&env);

    let explained = cli(&env, FOXHOUND)
        .args(["config", "explain", "politeness"])
        .args(["--set", "politeness.domain.concurrency=2", "--json"])
        .mode(mode)
        .json("SettingsExplanation")
        .unwrap();

    assert_eq!(explained.output.exit, Some(0), "{}", explained.output);
    snapshot_settings(&SnapshotValues::of(&env)).bind(|| {
        insta::assert_snapshot!(
            "config_explain_politeness",
            serde_json::to_string_pretty(&explained.documents[0]).unwrap()
        );
    });
}

/// Starts the Scripted engine as the Daemon of the test's home, playing
/// the corpus's `script`.
fn scripted_daemon(env: &TestEnv, script: &str) -> goodfetch_test_support::Process {
    let scripted = tools()
        .unwrap()
        .path(Tool::ScriptedEngine)
        .unwrap()
        .to_owned();
    let mut command = env.process_command(scripted);
    command
        .arg("--script")
        .arg(engine_script(script))
        .args(["--mode", "socket"])
        .stderr(Stdio::null());
    let process = env.spawn(&mut command).unwrap();
    reported_address(&mut process.child()).unwrap();
    process
}

/// Under `--json`, a followed crawl whose Connection drops once its Run ID
/// is printed keeps stdout to that Run ID alone, and prints the failure on
/// stderr as one JSON line naming the call that failed,
/// `run.progress`: `connection_lost`, exit `8` (ADR 0069).
///
/// Covers: story:11-1132, command:crawl.start, error:connection_lost, exit:unavailable
#[test]
fn a_failure_after_the_run_id_goes_to_stderr_naming_the_call_that_failed() {
    let env = TestEnv::new();
    let scripted = scripted_daemon(&env, "daemon/cli-crawl-follow-dropped.json");

    let output = cli(&env, FOXHOUND)
        .args([
            "crawl",
            "start",
            "https://example.com/",
            "--run-id",
            "first",
        ])
        .args(["--follow", "--json", "--daemon"])
        .run()
        .unwrap();
    scripted.child().wait().unwrap();
    let failure = output
        .stderr
        .lines()
        .find_map(|line| serde_json::from_str::<Value>(line).ok())
        .unwrap_or(Value::Null);

    assert_eq!(
        (
            output.exit,
            output.stdout.as_str(),
            failure["error"]["code"].clone(),
            failure["error"]["operation"].clone(),
        ),
        (
            Some(8),
            "\"first\"\n",
            json!("connection_lost"),
            json!("run.progress"),
        ),
        "{output}"
    );
}

/// `run export --out -` to a standard output that takes no bytes, a full
/// device, stops `export.stream` at its first chunk and exits `14`,
/// `file_write_failed`, rather than ending quietly or exporting on.
///
/// Covers: story:11-1132, command:run.export, error:file_write_failed, exit:file_write_failed
#[cfg(target_os = "linux")]
#[test]
fn run_export_to_a_full_standard_output_stops_the_call_and_exits_14() {
    const ARGUMENTS: [&str; 9] = [
        "run", "export", "first", "pages", "--format", "jsonl", "--out", "-", "--daemon",
    ];
    let env = TestEnv::new();
    let scripted = scripted_daemon(&env, "daemon/cli-run-export-stopped.json");
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .unwrap();
    // `TestEnv::spawn` pipes stdout, so the command runs here, its stdout
    // the full device; it ends with the call it makes.
    let mut command = env.process_command(FOXHOUND);
    let output = command
        .args(ARGUMENTS)
        .stdin(Stdio::null())
        .stdout(full)
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    record_command(&env, &ARGUMENTS, output.status.code()).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let played = scripted.child().wait().unwrap();

    assert_eq!(
        (
            output.status.code(),
            stderr.contains("code: file_write_failed"),
            played.success(),
        ),
        (Some(14), true, true),
        "{stderr}"
    );
}
