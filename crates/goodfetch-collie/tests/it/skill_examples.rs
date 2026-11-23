//! Collie's CLI skill's example commands (#1361), run as the rendered skill
//! writes them, one command after another from a shell's point of view,
//! against the local Proving ground: `run start` reaches the Daemon, which
//! it starts, and every later command reaches the same Daemon. Each
//! `<run-id>` and `<page-id>` is the one an earlier answer gave, and the
//! example's site is the Proving ground's `skill-examples` task, whose oracle
//! sees whether the clicks and keys reached the page. The test has its own
//! Goodfetch home, a cleared environment, its own Proving ground and a
//! Browser cache in its own folder (ADRs 0059 and 0094).
#![expect(
    clippy::disallowed_methods,
    reason = "the test polls the Proving ground's oracle on its own thread, never an async worker"
)]

use std::time::Duration;

use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, ProvingGround, TestEnv, ToolStatus, browser_cache, cli,
};
use serde_json::Value;

use crate::daemon::StopsTheDaemon;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Collie's CLI skill as `xtask codegen` renders it for the executable.
const SKILL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/generated/skills/goodfetch-collie-cli/SKILL.md"
));

/// The site the skill's examples name, which the test replaces with the
/// task's start URL.
const EXAMPLE_SITE: &str = "https://example.com/";

/// The Proving ground's task the examples carry out.
const TASK: &str = "actionability/30";

/// The heading the task's page shows, which the example's `page read`
/// reads.
const HEADING: &str = "Skill examples";

/// The Proving ground's own seed for this test.
const SEED: u64 = 1361;

/// How long the oracle is asked again for the Effect the page posts after
/// the key press.
const EFFECT_WAIT: Duration = Duration::from_secs(10);

/// Every command the skill's examples write: each `$ ` line of a `console`
/// block, as its words.
fn example_commands(markdown: &str) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    let mut console = false;
    let mut fenced = false;
    for line in markdown.lines() {
        if let Some(info) = line.trim_start().strip_prefix("```") {
            console = !fenced && info.trim() == "console";
            fenced = !fenced;
        } else if console && let Some(command) = line.strip_prefix("$ ") {
            commands.push(words(command));
        }
    }
    commands
}

/// `line`'s words as a POSIX shell splits them, for the quoting the
/// examples use: blanks between words, and single quotes around a word's
/// text taken as it is.
fn words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quoted = false;
    for character in line.chars() {
        match character {
            '\'' => {
                quoted = !quoted;
                word.get_or_insert_with(String::new);
            }
            ' ' | '\t' if !quoted => words.extend(word.take()),
            _ => word.get_or_insert_with(String::new).push(character),
        }
    }
    words.extend(word);
    words
}

/// The oracle's outcome for [`TASK`], done, asked again until it is
/// `verified` or [`EFFECT_WAIT`] passes, since the page posts its Effect
/// after its event.
fn outcome(proving_ground: &ProvingGround) -> Value {
    let started = std::time::Instant::now();
    loop {
        let verdict = proving_ground.judge(TASK, ToolStatus::Success).unwrap();
        if verdict["outcome"] == "verified" || started.elapsed() > EFFECT_WAIT {
            return verdict;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Run one after another as the skill writes them, the CLI skill's example
/// commands each succeed, the read answers the page's heading, and the
/// oracle verifies the clicks and keys before the Run finishes.
/// Covers: story:11-237
#[test]
fn the_cli_skills_example_commands_carry_out_their_task_on_the_proving_ground() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let env = env.with_variable(BROWSERS_PATH_VARIABLE, &cache.to_string_lossy());
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let resolve = &proving_ground.description().resolve;
    let hosts = resolve
        .keys()
        .map(|host| serde_json::to_string(host).unwrap())
        .collect::<Vec<_>>();
    let addresses = resolve
        .iter()
        .map(|(host, address)| {
            format!(
                "{} = [{}]",
                serde_json::to_string(host).unwrap(),
                serde_json::to_string(address).unwrap()
            )
        })
        .collect::<Vec<_>>();
    env.user_configuration(&format!(
        "network.resolve = {{ {} }}\negress.private_ranges = [{}]\n",
        addresses.join(", "),
        hosts.join(", ")
    ));
    let site = proving_ground.start_url(TASK).unwrap();
    let _stops = StopsTheDaemon::of(&env, COLLIE);

    let mut run_id = String::new();
    let mut page_id = String::new();
    let mut failed = Vec::new();
    let mut read = None;
    let mut verdict = None;
    for command in example_commands(SKILL) {
        let (executable, arguments) = command.split_first().unwrap();
        assert_eq!(executable, "goodfetch-collie", "{command:?}");
        if arguments.starts_with(&["run".to_owned(), "finish".to_owned()]) {
            verdict = Some(outcome(&proving_ground));
        }
        let arguments: Vec<String> = arguments
            .iter()
            .map(|word| {
                word.replace("<run-id>", &run_id)
                    .replace("<page-id>", &page_id)
                    .replace(EXAMPLE_SITE, &site)
            })
            .collect();
        let output = cli(&env, COLLIE).args(&arguments).run().unwrap();
        if output.exit != Some(0) {
            failed.push(format!("{arguments:?}: {output}"));
            continue;
        }
        let answer: Value = serde_json::from_str(&output.stdout)
            .unwrap_or_else(|error| panic!("{arguments:?}: {error}: {output}"));
        if let Some(id) = answer["run_id"].as_str() {
            id.clone_into(&mut run_id);
        }
        if let Some(id) = answer["page_id"].as_str() {
            id.clone_into(&mut page_id);
        }
        if arguments.starts_with(&["page".to_owned(), "read".to_owned()]) {
            read = Some(answer);
        }
    }

    assert_eq!(failed, Vec::<String>::new());
    let read = read.expect("the examples read the page");
    assert!(read.to_string().contains(HEADING), "{read}");
    let verdict = verdict.expect("the examples finish the Run");
    assert_eq!(verdict["outcome"], "verified", "{verdict}");
}
