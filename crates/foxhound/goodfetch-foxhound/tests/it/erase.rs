//! Exports and erasing a Dataset on Foxhound's executable (ADRs 0031, 0103
//! and 0116; spec #1937, issue 1220), each test on its own home and the
//! Daemon of it: `run export` writes an Export in every format the engine
//! writes and fetches its file by the Export's handle; `dataset erase`,
//! asked in a pseudo-terminal, erases a Dataset of a run no engine runs at
//! once, so a later read finds it gone, and is refused with
//! `person_required` where no person answers: under `--json`, from a pipe,
//! from a script, and over the protocol from every surface but a person's.
#![expect(
    clippy::disallowed_methods,
    reason = "a test reads the files its commands write"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_test_client::{Attached, Hello, HomeLayout};
use goodfetch_test_support::{CliOutput, ProvingGround, TestEnv, cli, pty};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::FOXHOUND;
use crate::crawl_commands::{crawl_start, settings_file, short_idle, start_home_daemon};

/// The Proving ground's seed.
const SEED: u64 = 1220;

/// The task whose index links Markdown pages.
const MARKDOWN_TASK: &str = "extraction/7";

/// A word the index's Markdown holds.
const WORD: &str = "fenced";

/// The formats the engine writes an Export in.
const FORMATS: [&str; 12] = [
    "json",
    "jsonl",
    "csv",
    "xlsx",
    "xml",
    "parquet",
    "html_table",
    "rss",
    "atom",
    "avro",
    "arrow_ipc",
    "markdown",
];

/// Crawls the index of the Markdown task as the code-free run `run_id` on
/// the Daemon of `env`'s home, and waits until the run ended, its one
/// Record of `pages` committed.
fn crawled_index(env: &TestEnv, proving_ground: &ProvingGround, run_id: &str) {
    let settings = settings_file(env, proving_ground);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    crawl_start(env, &settings, run_id, &[index]);
    // The crawl has ended once its last ending is read.
    let followed = cli(env, FOXHOUND)
        .args(["run", "outcomes", run_id, "--follow", "--json"])
        .run()
        .unwrap();
    assert_eq!(followed.exit, Some(0), "{followed}");
}

/// How many Records of `pages` of `run_id` each read finds: a page of the
/// Dataset, `run records` and a search for [`WORD`].
fn records_read(env: &TestEnv, run_id: &str) -> [usize; 3] {
    let page = cli(env, FOXHOUND)
        .args(["dataset", "page", "--run-id", run_id, "--dataset", "pages"])
        .arg("--json")
        .json("DatasetPage")
        .unwrap();
    let records = cli(env, FOXHOUND)
        .args(["run", "records", run_id, "--dataset", "pages", "--json"])
        .json_array("RecordRead")
        .unwrap();
    let searched = cli(env, FOXHOUND)
        .args([
            "dataset",
            "search",
            "--run-id",
            run_id,
            "--dataset",
            "pages",
        ])
        .args(["--text", WORD, "--json"])
        .json("DatasetSearchPage")
        .unwrap();
    let count = |value: &Value| value.as_array().map_or(usize::MAX, Vec::len);
    [
        count(&page.documents[0]["records"]),
        count(&records.documents[0]),
        count(&searched.documents[0]["records"]),
    ]
}

/// The run folder's Exports, which a run's `run export` keeps.
fn exports_kept(env: &TestEnv, run_id: &str) -> bool {
    HomeLayout::of(env.home())
        .run_folder(run_id)
        .join("exports")
        .exists()
}

/// An Export of `pages` starts from the command line in each format the
/// engine writes, on the Daemon where the crawl ran, and `run export`
/// fetches its file by the Export's handle: the file it writes is the
/// Export's size and SHA-256, and holds the run's one Record.
///
/// Covers: story:12-213, command:run.export
#[test]
fn an_export_starts_from_the_command_line_in_each_format_and_is_fetched_by_handle() {
    let env = TestEnv::new();
    short_idle(&env);
    let _daemon = start_home_daemon(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    crawled_index(&env, &proving_ground, "exported");

    let exported: Vec<(&str, Option<i32>, Value, bool)> = FORMATS
        .iter()
        .map(|&format| {
            let path = env.folder().join(format!("pages.{format}"));
            let export = cli(&env, FOXHOUND)
                .args(["run", "export", "exported", "pages", "--format", format])
                .args(["--out", path.to_str().unwrap(), "--json"])
                .json("Export")
                .unwrap();
            let bytes = std::fs::read(&path).unwrap_or_default();
            let answered = &export.documents[0];
            let fetched = answered["size"] == json!(bytes.len())
                && answered["sha256"] == json!(format!("{:x}", Sha256::digest(&bytes)));
            (
                format,
                export.output.exit,
                answered["records"].clone(),
                fetched,
            )
        })
        .collect();

    let expected: Vec<(&str, Option<i32>, Value, bool)> = FORMATS
        .iter()
        .map(|&format| (format, Some(0), json!(1), true))
        .collect();
    assert_eq!(exported, expected);
}

/// Erasing a Dataset applies at once, asked in a pseudo-terminal once,
/// after its warning, on a run no engine runs: a yes erases every Record,
/// so a page of the Dataset, `run records` and a search all find none, and
/// the run's kept Exports go with them; a no erases nothing.
///
/// Covers: story:12-214, command:dataset.erase
#[rstest::rstest]
#[case::answered_yes("y", [0, 0, 0], false)]
#[case::answered_no("n", [1, 1, 1], true)]
fn erasing_a_dataset_at_the_terminal_applies_at_once(
    #[case] answer: &str,
    #[case] read: [usize; 3],
    #[case] exports: bool,
) {
    let env = TestEnv::new();
    short_idle(&env);
    let _daemon = start_home_daemon(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    crawled_index(&env, &proving_ground, "erased");
    let kept = env.folder().join("kept.jsonl");
    let export = cli(&env, FOXHOUND)
        .args(["run", "export", "erased", "pages", "--format", "jsonl"])
        .args(["--out", kept.to_str().unwrap()])
        .run()
        .unwrap();
    assert_eq!(export.exit, Some(0), "{export}");
    let before = records_read(&env, "erased");

    let mut erase = pty(&env, FOXHOUND)
        .args([
            "dataset",
            "erase",
            "--run-id",
            "erased",
            "--dataset",
            "pages",
        ])
        .spawn()
        .unwrap();
    erase
        .expect("Erase the Dataset `pages` of the run `erased`? [y/N]")
        .unwrap();
    erase.send_line(answer).unwrap();
    let erased = erase.finish().unwrap();

    assert_eq!(
        (
            before,
            erased.exit,
            erased.stripped().contains("cannot be undone"),
            records_read(&env, "erased"),
            exports_kept(&env, "erased"),
        ),
        ([1, 1, 1], Some(0), true, read, exports),
        "{}",
        erased.stripped()
    );
}

/// The code, whether it is retryable, and the `fix` of a command's
/// refusal: one `CommandFailed` on stdout under `--json`, else its lines
/// on stderr.
fn refusal(output: &CliOutput, json: bool) -> (Value, Value, String) {
    if json {
        let printed: Value =
            serde_json::from_str(&output.stdout).unwrap_or_else(|_| panic!("{output}"));
        let error = &printed["error"];
        return (
            error["code"].clone(),
            error["retryable"].clone(),
            error["fix"].as_str().unwrap_or_default().to_owned(),
        );
    }
    let line = |label: &str| {
        output
            .stderr
            .lines()
            .find_map(|line| line.strip_prefix(label))
            .unwrap_or_default()
            .to_owned()
    };
    (
        json!(line("code: ")),
        json!(line("retryable: ") == "true"),
        line("fix: "),
    )
}

/// Where no person answers, under `--json`, from a pipe and from a script,
/// as an agent or CI runs it, `dataset erase` is refused with
/// `person_required`, not retryable, and exits `4` before any engine call,
/// its `fix` naming the interactive command; the Dataset keeps its Record.
///
/// Covers: story:12-214, command:dataset.erase, exit:refused
#[test]
fn erasing_a_dataset_is_refused_where_no_person_answers() {
    let env = TestEnv::new();
    short_idle(&env);
    let _daemon = start_home_daemon(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    crawled_index(&env, &proving_ground, "kept");

    let ways: [(bool, Option<&str>); 3] =
        [(true, Some("y\n")), (false, Some("y\n")), (false, None)];
    let refused: Vec<(Option<i32>, Value, Value, bool)> = ways
        .iter()
        .map(|&(json, stdin)| {
            let mut erase = cli(&env, FOXHOUND).args([
                "dataset",
                "erase",
                "--run-id",
                "kept",
                "--dataset",
                "pages",
            ]);
            if json {
                erase = erase.arg("--json");
            }
            if let Some(stdin) = stdin {
                erase = erase.stdin(stdin);
            }
            let output = erase.run().unwrap();
            let (code, retryable, fix) = refusal(&output, json);
            (
                output.exit,
                code,
                retryable,
                fix.contains("`dataset erase`"),
            )
        })
        .collect();

    assert_eq!(
        (refused, records_read(&env, "kept")),
        (
            vec![(Some(4), json!("person_required"), json!(false), true); 3],
            [1, 1, 1]
        )
    );
}

/// An Attached engine of `env`'s home whose `hello` comes from `surface`.
fn engine_from(env: &TestEnv, surface: Surface) -> Attached {
    let mut engine = Attached::start(env, FOXHOUND);
    let opened = engine.open(
        &Hello::needing(Product::Foxhound)
            .surface(surface)
            .id("hello"),
    );
    assert_eq!(opened["result"]["product"], "foxhound", "{opened}");
    engine
}

/// The Error code of a reply, or `answered` for a result.
fn outcome(reply: &Value) -> String {
    match &reply["error"]["data"]["code"] {
        Value::String(code) => code.clone(),
        _ if reply["result"].is_object() => "answered".to_owned(),
        _ => panic!("{reply}"),
    }
}

/// `dataset.erase`'s params for `dataset` of `run_id`.
fn erase(run_id: &str, dataset: &str) -> Value {
    json!({ "run_id": run_id, "dataset": dataset })
}

/// Over the protocol, `dataset.erase` is refused with `person_required` to
/// a Client acting for a model (MCP), to each program's SDK and to a Rust
/// program, and the Dataset keeps its Records; a person's Connection, the
/// CLI's or the Kennel's, is answered: on a run its own engine runs with
/// `run_live`, on a run or Dataset that does not exist with
/// `run_not_found` and `dataset_unknown`, and on the ended run by erasing
/// every Record, which a later read finds gone.
///
/// Covers: story:12-214, operation:dataset.erase, error:person_required, error:run_live, error:run_not_found, error:dataset_unknown
#[test]
fn dataset_erase_is_refused_to_every_surface_but_a_persons() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let settings =
        crate::fetch_tiers::settings(&proving_ground, &json!({ "fetch": { "tier": "http" } }));
    let mut person = crate::opened_by(
        Attached::start(&env, FOXHOUND),
        Hello::needing(Product::Foxhound).surface(Surface::Cli),
    )
    .settings_by_default(&settings);
    crate::search::crawled(&mut person, &proving_ground, "shop", 3, &json!({}));

    let refused: Vec<String> = [
        Surface::Mcp,
        Surface::SdkTypescript,
        Surface::SdkPython,
        Surface::Rust,
    ]
    .into_iter()
    .map(|surface| {
        let mut engine = engine_from(&env, surface);
        let reply = engine.ask("erase", "dataset.erase", &erase("shop", "books"));
        assert!(engine.finished());
        outcome(&reply)
    })
    .collect();
    let kept = person.call(
        "page",
        "dataset.page",
        &json!({ "run_id": "shop", "dataset": "books" }),
        &mut |_| None,
    );

    let mut answer = |_: &Value| Some(json!({ "result": { "records": [] } }));
    let started = person.call(
        "live",
        "crawl.start",
        &json!({
            "run_id": "live",
            "seeds": [{ "url": proving_ground.url("shop.test", "/1"), "label": "page" }],
            "datasets": [{ "name": "books" }],
            "settings": { "crawl": { "keep_alive": true } },
            "labels": { "page": {} },
        }),
        &mut answer,
    );
    assert_eq!(started["result"]["run_id"], "live", "{started}");
    let live = person.call(
        "erase",
        "dataset.erase",
        &erase("live", "books"),
        &mut answer,
    );
    let finished = person.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "live" }),
        &mut answer,
    );
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    let missing = [erase("never-ran", "books"), erase("shop", "nope")]
        .map(|params| outcome(&person.call("erase", "dataset.erase", &params, &mut |_| None)));
    let erased = person.call(
        "erase",
        "dataset.erase",
        &erase("shop", "books"),
        &mut |_| None,
    );
    let after = person.call(
        "page",
        "dataset.page",
        &json!({ "run_id": "shop", "dataset": "books" }),
        &mut |_| None,
    );
    assert!(person.finished());
    let mut kennel = engine_from(&env, Surface::Kennel);
    let again = kennel.ask("erase", "dataset.erase", &erase("shop", "books"));
    assert!(kennel.finished());

    let count = |page: &Value| {
        page["result"]["records"]
            .as_array()
            .map_or(usize::MAX, Vec::len)
    };
    assert_eq!(
        (
            refused,
            count(&kept),
            outcome(&live),
            missing,
            erased["result"].clone(),
            count(&after),
            again["result"]["records"].clone(),
        ),
        (
            vec!["person_required".to_owned(); 4],
            3,
            "run_live".to_owned(),
            ["run_not_found".to_owned(), "dataset_unknown".to_owned()],
            json!({ "run_id": "shop", "dataset": "books", "records": 3, "rejected_records": 0 }),
            0,
            json!(0),
        )
    );
}

/// `run erase` is offered only as an interactive terminal prompt: refused with
/// `person_required` (exit 4) under `--json`, from a pipe or from a script
/// before any engine call, and when confirmed at a pseudo-terminal erases
/// matching Records, deletes kept Exports and records a `run_data_erased`
/// Audit log entry (ADR 0016; ticket 694).
///
/// Covers: story:4-120, command:run.erase, exit:refused
#[test]
fn run_erase_cli_prompt_applies_at_terminal_and_refuses_non_person_calls() {
    let env = TestEnv::new();
    short_idle(&env);
    let _daemon = start_home_daemon(&env);
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    crawled_index(&env, &proving_ground, "run-erased");
    let kept = env.folder().join("kept-run.jsonl");
    let export = cli(&env, FOXHOUND)
        .args(["run", "export", "run-erased", "pages", "--format", "jsonl"])
        .args(["--out", kept.to_str().unwrap()])
        .run()
        .unwrap();
    assert_eq!(export.exit, Some(0), "{export}");

    let ways: [(bool, Option<&str>); 3] =
        [(true, Some("y\n")), (false, Some("y\n")), (false, None)];
    let refused: Vec<(Option<i32>, Value, Value, bool)> = ways
        .iter()
        .map(|&(json, stdin)| {
            let mut erase =
                cli(&env, FOXHOUND).args(["run", "erase", "run-erased", "--value", WORD]);
            if json {
                erase = erase.arg("--json");
            }
            if let Some(stdin) = stdin {
                erase = erase.stdin(stdin);
            }
            let output = erase.run().unwrap();
            let (code, retryable, fix) = refusal(&output, json);
            (output.exit, code, retryable, fix.contains("`run erase`"))
        })
        .collect();
    assert_eq!(
        (refused, records_read(&env, "run-erased")),
        (
            vec![(Some(4), json!("person_required"), json!(false), true); 3],
            [1, 1, 1]
        )
    );

    let mut erase = pty(&env, FOXHOUND)
        .args(["run", "erase", "run-erased", "--value", WORD])
        .spawn()
        .unwrap();
    erase
        .expect("Erase matching data from the run `run-erased`? [y/N]")
        .unwrap();
    erase.send_line("y").unwrap();
    let erased = erase.finish().unwrap();

    let audit_out = cli(&env, FOXHOUND)
        .args(["audit", "export"])
        .run()
        .unwrap()
        .stdout;
    let has_audit = audit_out
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .any(|entry| {
            entry["kind"] == json!("run_data_erased")
                && entry["details"]["operation"] == json!("run.erase")
        });

    assert_eq!(
        (
            erased.exit,
            erased.stripped().contains("cannot be undone"),
            records_read(&env, "run-erased"),
            exports_kept(&env, "run-erased"),
            has_audit,
        ),
        (Some(0), true, [0, 0, 0], false, true),
        "{}",
        erased.stripped()
    );
}
