//! An older engine of either Product opens what the current Foxhound wrote
//! in the Goodfetch home they share (ADRs 0066, 0094 and 0098), Foxhound's
//! own entries among it. A skew job names the older executable, a released
//! minor's, in `GOODFETCH_TEST_OLDER_ENGINE`; with none the commit's own
//! executable of each Product stands for it. What an engine makes of each
//! file is the format registry's compatibility functions' answer, and the
//! older engine is driven by its commands alone, since another minor's
//! `hello` may be refused.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the Goodfetch home's Operator policy and read its files as they are"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::formats::{Compatibility, FORMATS, Format, RUN_INDEX, RUN_STATE};
use goodfetch_protocol::home::{INSTALLED_EXECUTABLES, POLITENESS_FILE};
use goodfetch_protocol::releases::ReleasesFile;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{
    CliOutput, DAEMON_DEADLINE, Daemon, Mode, ProvingGround, TestEnv, Tool, cli,
    lay_out_browser_entry, older_engine, sql_dump, tools,
};
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::fetch_tiers::settings;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

const SEED: u64 = 439;

/// The releases skew is tested against, with the format versions each
/// shipped.
const RELEASES: &str = include_str!("../../../../../compat/releases.toml");

/// The user configuration the current Foxhound's user set: a setting both
/// Products carry and one of Foxhound's own.
const CONFIGURATION: &str = "daemon.idle_timeout = 60000\ncrawl.max_requests = 50\n";

/// The Operator policy of the machine, the same way.
const POLICY: &str = concat!(
    "version = \"0.1\"\n",
    "[cap.crawl]\n",
    "max_requests = 100\n",
    "[lock.run_state.batch]\n",
    "pages = 7\n",
    "[lock.site_preferences]\n",
    "respect_robots_txt = true\n",
);

/// The host whose certificate the fixture's crawl pins at first use.
const PINNED_HOST: &str = "expired.certificates.test";

/// The older engine a test drives, and what it reads.
struct Older {
    executable: PathBuf,
    /// Its Product version's major and minor.
    version: ProtocolVersion,
    /// The version of each registered format it ships.
    formats: BTreeMap<String, ProtocolVersion>,
}

impl Older {
    /// The older Collie.
    fn collie() -> Self {
        Self::of(
            "goodfetch-collie",
            tools().unwrap().path(Tool::Collie).unwrap(),
        )
    }

    /// The older Foxhound.
    fn foxhound() -> Self {
        Self::of("goodfetch-foxhound", Path::new(FOXHOUND))
    }

    /// The older engine standing in for `built`, whose `--version` names
    /// `name`: a job whose variable names the other Product's executable
    /// fails here.
    fn of(name: &str, built: &Path) -> Self {
        let executable = older_engine(built);
        let printed = cli(&TestEnv::new(), &executable)
            .arg("--version")
            .run()
            .unwrap()
            .stdout;
        let (product, version) = printed.trim().split_once(' ').unwrap();
        assert_eq!(product, name, "{}", executable.display());
        Self {
            formats: formats_of(version),
            version: major_minor(version),
            executable,
        }
    }

    /// Its command `arguments`, run in its own process.
    fn run(&self, env: &TestEnv, arguments: &[&str]) -> CliOutput {
        cli(env, &self.executable)
            .args(arguments)
            .mode(Mode::Attached)
            .run()
            .unwrap()
    }

    /// The JSON value its command `arguments` printed, a command that never
    /// reaches an engine, so takes no `--attached`.
    fn json_alone(&self, env: &TestEnv, arguments: &[&str]) -> Value {
        answered(&cli(env, &self.executable).args(arguments).run().unwrap())
    }

    /// The JSON value its command `arguments` printed, which succeeded.
    fn json(&self, env: &TestEnv, arguments: &[&str]) -> Value {
        answered(&self.run(env, arguments))
    }

    /// What it makes of the run file `run_file`.
    fn run_file_verdict(&self, run_file: &Path) -> Compatibility {
        let found: String = Connection::open(run_file)
            .unwrap()
            .query_row(
                "SELECT version FROM format WHERE name = ?1",
                [RUN_STATE.name],
                |row| row.get(0),
            )
            .unwrap();
        RUN_STATE
            .rule
            .compatibility(self.reads(&RUN_STATE), found.parse().unwrap())
    }

    fn reads(&self, format: &Format) -> ProtocolVersion {
        self.formats[format.name]
    }

    /// What it makes of the Run index of `env`'s home.
    fn run_index_verdict(&self, env: &TestEnv) -> Compatibility {
        let index = Connection::open(HomeLayout::of(env.home()).run_index()).unwrap();
        let (found, lowest_minor): (String, u16) = index
            .query_row("SELECT version, lowest_minor FROM format", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        RUN_INDEX.rule.compatibility_of_store(
            self.reads(&RUN_INDEX),
            found.parse().unwrap(),
            self.version,
            lowest_minor,
        )
    }
}

/// The version of each registered format the Product version `version`
/// ships: the list's for a release, the registry's for the commit itself.
///
/// # Panics
///
/// When `version` is neither.
fn formats_of(version: &str) -> BTreeMap<String, ProtocolVersion> {
    let file: ReleasesFile = toml::from_str(RELEASES).unwrap();
    let released = file
        .release
        .iter()
        .find(|release| String::from(release.version) == version);
    if let Some(release) = released {
        return release
            .formats
            .iter()
            .map(|(name, version)| {
                (
                    name.as_str().to_owned(),
                    String::from(*version).parse().unwrap(),
                )
            })
            .collect();
    }
    assert_eq!(
        version,
        env!("CARGO_PKG_VERSION"),
        "the older engine is no release of compat/releases.toml"
    );
    FORMATS
        .iter()
        .map(|format| (format.name.to_owned(), format.version))
        .collect()
}

/// A Product version's major and minor.
fn major_minor(version: &str) -> ProtocolVersion {
    let mut numbers = version.split('.');
    let mut next = || numbers.next().unwrap().parse().unwrap();
    ProtocolVersion {
        major: next(),
        minor: next(),
    }
}

/// The one JSON value `output` printed, its command having succeeded.
fn answered(output: &CliOutput) -> Value {
    assert_eq!(output.exit, Some(0), "{output:?}");
    serde_json::from_str(&output.stdout).unwrap()
}

/// The JSON value the current Foxhound's command `arguments` printed, run
/// in its own process.
fn foxhound(env: &TestEnv, arguments: &[&str]) -> Value {
    answered(
        &cli(env, FOXHOUND)
            .args(arguments)
            .mode(Mode::Attached)
            .run()
            .unwrap(),
    )
}

/// The current Foxhound's `crawl start` of `url` as the Run `run_id`,
/// followed to its end, its pages read over HTTP with no delay.
fn crawl_arguments(
    env: &TestEnv,
    proving_ground: &ProvingGround,
    (run_id, url): (&str, &str),
) -> Vec<String> {
    let path = env.folder().join(env.unique_name("settings") + ".json");
    let more = json!({
        "fetch": { "tier": "http" },
        "politeness": { "domain": { "delay": 0 } },
        "tls": { "accept_invalid_certs": { "certificates.test": null } },
    });
    std::fs::write(&path, settings(proving_ground, &more).to_string()).unwrap();
    [
        "crawl",
        "start",
        url,
        "--run-id",
        run_id,
        "--links",
        "none",
        "--format",
        "markdown",
        "--settings",
        path.to_str().unwrap(),
        "--follow",
        "--json",
    ]
    .map(str::to_owned)
    .into()
}

/// The Goodfetch home the current Foxhound wrote: user configuration, the
/// Operator policy, the Usage ledger with an install Limit of five
/// requests, the Audit log, the Politeness file and the known certificates
/// of a crawl of a host whose certificate it pinned, the Run index, and
/// the list of installed executables, where its Daemon recorded it.
fn written_by_foxhound(env: &TestEnv, proving_ground: &ProvingGround) {
    env.user_configuration(CONFIGURATION);
    std::fs::write(HomeLayout::of(env.home()).operator_policy(), POLICY).unwrap();
    foxhound(
        env,
        &[
            "usage",
            "set",
            "--scope",
            "install",
            "--usage-kind",
            "requests",
            "--amount",
            "5",
            "--json",
        ],
    );
    let url = format!("https://{PINNED_HOST}:{}/", proving_ground.address().port());
    let arguments = crawl_arguments(env, proving_ground, ("pinning", &url));
    let crawled = cli(env, FOXHOUND)
        .args(&arguments)
        .mode(Mode::Attached)
        .run()
        .unwrap();
    assert_eq!(crawled.exit, Some(0), "{crawled:?}");
    Daemon::start_program(env, Path::new(FOXHOUND), DAEMON_DEADLINE)
        .unwrap()
        .stop(env)
        .unwrap();
}

/// The entry the list of installed executables holds of the current
/// Foxhound.
fn installed_foxhound(env: &TestEnv) -> Value {
    let list = std::fs::read_to_string(env.home().join(INSTALLED_EXECUTABLES)).unwrap();
    let list: Value = serde_json::from_str(&list).unwrap();
    let executable = std::fs::canonicalize(FOXHOUND).unwrap();
    list["executables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| {
            let listed = entry["path"].as_str().unwrap();
            std::fs::canonicalize(listed).is_ok_and(|listed| listed == executable)
        })
        .cloned()
        .unwrap()
}

/// The browser build the current Foxhound pins, as that entry says.
fn pinned_build(entry: &Value) -> String {
    entry["pins"][0]["build"].as_str().unwrap().to_owned()
}

/// What no engine rewrites as it opens the home: user configuration, the
/// Operator policy and the Politeness file, whose rows are Foxhound's own.
fn unchanged_files(env: &TestEnv) -> [String; 3] {
    let layout = HomeLayout::of(env.home());
    let politeness = env.home().join(POLITENESS_FILE);
    [
        std::fs::read_to_string(layout.user_configuration()).unwrap(),
        std::fs::read_to_string(layout.operator_policy()).unwrap(),
        sql_dump(&Connection::open(politeness).unwrap()).unwrap(),
    ]
}

/// The Run IDs `listed`, a `run list --json`, holds.
fn run_ids(listed: &Value) -> Vec<&str> {
    let mut ids: Vec<&str> = listed["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| run["run_id"].as_str().unwrap())
        .collect();
    ids.sort_unstable();
    ids
}

/// Each Limit's amount and charge, as a `usage list --json` says.
fn charged_limits(listed: &Value) -> Vec<[&Value; 2]> {
    listed["limits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|limit| [&limit["limit"]["amount"], &limit["charged"]])
        .collect()
}

/// Each pinned certificate's host and key, as a `certs list --json` says.
fn pinned_certificates(listed: &Value) -> Vec<[&Value; 2]> {
    listed["pins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pin| [&pin["host"], &pin["spki_sha256"]])
        .collect()
}

/// The older engine's commands read each store of the home the current
/// Foxhound wrote, its Runs use it, and Foxhound's own entries stay.
#[rstest]
#[case::an_older_collie(Older::collie())]
#[case::an_older_foxhound(Older::foxhound())]
fn an_older_engine_opens_the_home_the_current_foxhound_wrote(#[case] older: Older) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    written_by_foxhound(&env, &proving_ground);
    let entry = installed_foxhound(&env);
    let cache = env.home().join("browsers");
    let pinned = lay_out_browser_entry(&cache, &pinned_build(&entry), "2020-01-01T00:00:00Z");
    let unpinned = lay_out_browser_entry(&cache, "pinned-by-nothing", "2020-01-01T00:00:00Z");
    let written = unchanged_files(&env);
    let limits = foxhound(&env, &["usage", "list", "--json"]);
    let pins = foxhound(&env, &["certs", "list", "--json"]);
    let index = older.run_index_verdict(&env);

    let older_limits = older.json(&env, &["usage", "list", "--json"]);
    let older_pins = older.json(&env, &["certs", "list", "--json"]);
    let audit = older.json(&env, &["audit", "verify", "--json"]);
    let explained = older.json(
        &env,
        &["config", "explain", "daemon", "run_state", "--json"],
    );
    older.json_alone(&env, &["doctor", "--json"]);
    let cleaned = older.json_alone(&env, &["browser", "clean", "--json"]);
    let listed = older.run(&env, &["run", "list", "--json"]);
    let started = older.run(&env, &["run", "start", "--run-id", "older", "--json"]);

    let charged = charged_limits(&limits);
    assert_eq!(charged_limits(&older_limits), charged);
    assert_eq!(charged[0][0], &json!(5));
    let certificates = pinned_certificates(&pins);
    assert_eq!(pinned_certificates(&older_pins), certificates);
    assert_eq!(certificates[0][0], &json!(PINNED_HOST));
    assert_eq!(audit["intact"], json!(true), "{audit}");
    let layers: BTreeMap<&str, &str> = explained["settings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|setting| {
            (
                setting["key"].as_str().unwrap(),
                setting["layer"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        (
            layers["daemon.idle_timeout"],
            layers["run_state.batch.pages"]
        ),
        ("user_configuration", "operator_policy")
    );
    assert_eq!(
        (
            cleaned["removed"][0]["version"].clone(),
            cleaned["removed"].as_array().map(Vec::len),
            pinned.join("entry.json").exists(),
            unpinned.join("entry.json").exists()
        ),
        (json!("pinned-by-nothing"), Some(1), true, false),
        "{cleaned}"
    );
    match index {
        Compatibility::Compatible => {
            assert_eq!(run_ids(&answered(&listed)), ["pinning"]);
            answered(&started);
        }
        _ => assert!(
            listed.stdout.contains("format_version_unsupported"),
            "{listed:?}"
        ),
    }
    assert_eq!(unchanged_files(&env), written);
    assert_eq!(installed_foxhound(&env)["pins"], entry["pins"]);
    assert_eq!(foxhound(&env, &["certs", "list", "--json"]), pins);
    let runs = foxhound(&env, &["run", "list", "--json"]);
    assert_eq!(
        run_ids(&runs),
        match index {
            Compatibility::Compatible => vec!["older", "pinning"],
            _ => vec!["pinning"],
        }
    );
}

/// The older engine finds the Daemon the current Foxhound runs, by the
/// runtime file both Products share, and names its Product.
#[rstest]
#[case::an_older_collie(Older::collie())]
#[case::an_older_foxhound(Older::foxhound())]
fn an_older_engine_finds_the_current_foxhounds_daemon(#[case] older: Older) {
    let env = TestEnv::new();
    // Used once, the current Foxhound is listed as installed, so its Daemon
    // is its major's pick and hands over to no older engine.
    foxhound(&env, &["usage", "list", "--json"]);
    let daemon = Daemon::start_program(&env, Path::new(FOXHOUND), DAEMON_DEADLINE).unwrap();

    let status = older.json_alone(&env, &["daemon", "status", "--json"]);

    let pid = daemon.pid();
    daemon.stop(&env).unwrap();
    assert_eq!(
        (&status["product"], &status["pid"]),
        (&json!("foxhound"), &json!(pid)),
        "{status}"
    );
}

/// The older engine resumes a Run the current Foxhound started and left
/// interrupted, from its run file, when it reads that file's format and the
/// Run index's, and refuses it untouched when it does not.
#[rstest]
#[case::an_older_collie(Older::collie())]
#[case::an_older_foxhound(Older::foxhound())]
fn an_older_engine_resumes_a_run_the_current_foxhound_left(#[case] older: Older) {
    let env = TestEnv::new();
    let started = foxhound(&env, &["run", "start", "--run-id", "first", "--json"]);
    assert_eq!(started["started_as"], json!("new"), "{started}");
    let run_file = HomeLayout::of(env.home()).run_file("first");
    let readable = (
        older.run_file_verdict(&run_file),
        older.run_index_verdict(&env),
    ) == (Compatibility::Compatible, Compatibility::Compatible);
    let written = sql_dump(&Connection::open(&run_file).unwrap()).unwrap();

    let resumed = older.run(&env, &["run", "resume", "first", "--json"]);

    if readable {
        assert_eq!(answered(&resumed)["started_as"], json!("resumed"));
        let runs = foxhound(&env, &["run", "list", "--json"]);
        assert_eq!(run_ids(&runs), ["first"]);
    } else {
        assert!(
            resumed.stdout.contains("format_version_unsupported"),
            "{resumed:?}"
        );
        assert_eq!(
            sql_dump(&Connection::open(&run_file).unwrap()).unwrap(),
            written
        );
    }
}

/// An older Foxhound reads the Datasets and seeds of a crawl the current
/// Foxhound ran, the tables Foxhound adds to the run file, as the current
/// one does.
#[test]
fn an_older_foxhound_reads_the_current_foxhounds_crawl() {
    let older = Older::foxhound();
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let url = format!("https://{PINNED_HOST}:{}/", proving_ground.address().port());
    let arguments = crawl_arguments(&env, &proving_ground, ("crawled", &url));
    let crawled = cli(&env, FOXHOUND)
        .args(&arguments)
        .mode(Mode::Attached)
        .run()
        .unwrap();
    assert_eq!(crawled.exit, Some(0), "{crawled:?}");
    let reads = [
        ["dataset", "list", "--run-id", "crawled", "--json"].as_slice(),
        ["run", "seeds", "crawled", "--json"].as_slice(),
    ];
    let current: Vec<Value> = reads.iter().map(|read| foxhound(&env, read)).collect();
    let verdict = older.run_file_verdict(&HomeLayout::of(env.home()).run_file("crawled"));

    let read: Vec<CliOutput> = reads.iter().map(|read| older.run(&env, read)).collect();

    match verdict {
        Compatibility::Compatible => {
            let read: Vec<Value> = read.iter().map(answered).collect();
            assert_eq!(read, current);
            assert_eq!(current[0]["datasets"].as_array().map(Vec::len), Some(1));
        }
        _ => {
            for output in &read {
                assert!(
                    output.stdout.contains("format_version_unsupported"),
                    "{output:?}"
                );
            }
        }
    }
}
