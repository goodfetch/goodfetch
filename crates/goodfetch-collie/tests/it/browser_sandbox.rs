//! Chromium's sandbox through Collie's executable (spec #564, ticket
//! #642): on by default, its renderers sandboxed; the startup probe's
//! refusal, naming the environment and its fix, on each host where a fix
//! exists, beside the same run on the host with the fix applied; the
//! sandbox switched off and reported on a platform that allows none; and
//! the Operator policy's switch, which a Client's own is refused.
//!
//! The hosts are made with `goodfetch-test-support`'s `under_default_seccomp`
//! (Docker's and containerd's default seccomp rules for namespaces, and a
//! Kubernetes pod's through its service variable) and
//! `under_apparmor_restriction` (Ubuntu's `unprivileged_userns` profile,
//! as a browser no AppArmor profile gives `userns` runs under), on Linux,
//! where the boundary table schedules them (TESTING.md, Confinement).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads `/proc`, lists processes and writes the policy on its own thread, never an async worker"
)]

use std::collections::BTreeSet;
use std::process::Command;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{
    ProvingGround, TestEnv, apparmor_restricts_user_namespaces, browser_cache, cli,
    under_apparmor_restriction, under_default_seccomp,
};
use rstest::rstest;
use serde_json::{Map, Value, json};

use crate::page::{browser_command_lines, opened, started_with};
use goodfetch_test_client::{Attached, hello};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The Proving ground's own seed for these tests.
const SEED: u64 = 642;

/// What a Run that opened one page on a host saw: `run.new_page`'s answer,
/// its report, the command lines of its browsers and the engine's stderr.
struct Seen {
    opened: Value,
    report: Value,
    command_lines: Vec<String>,
    stderr: String,
}

/// Opens a page in the Run `run_id` of an engine whose command `change`
/// changed, then finishes the Run.
fn open_a_page(env: &TestEnv, run_id: &str, change: impl FnOnce(&mut Command)) -> Seen {
    let proving_ground = ProvingGround::start(env, SEED).unwrap();
    let cache = browser_cache(env).unwrap();
    // The page navigates nowhere, so the Run lists no private range, which
    // would join the report's `weakened`.
    let settings = json!({ "egress": { "private_ranges": [] } });
    let settings: Map<String, Value> = settings.as_object().cloned().unwrap_or_default();
    let mut engine = started_with(env, &cache, &proving_ground, run_id, &settings, change);
    let opened = engine.ask(2, "run.new_page", &json!({ "run_id": run_id }));
    let command_lines = browser_command_lines(env)
        .into_iter()
        .filter(|line| !line.contains("--type="))
        .collect();
    let report = engine.ask(3, "run.finish", &json!({ "run_id": run_id }));
    let (_, stderr) = engine.finish_with_stderr();
    Seen {
        opened,
        report: report["result"].clone(),
        command_lines,
        stderr,
    }
}

/// The host each environment case makes.
fn host(environment: &str) -> fn(&mut Command) {
    match environment {
        "docker_default_seccomp" => under_default_seccomp,
        "kubernetes_runtime_default" => |command| {
            under_default_seccomp(command);
            command.env("KUBERNETES_SERVICE_HOST", "10.96.0.1");
        },
        "apparmor_userns" => under_apparmor_restriction,
        _ => unreachable!("no such host"),
    }
}

/// The same host with its fix applied: a seccomp profile that allows the
/// namespace, which is the host's own rules, or Chrome for Testing's
/// AppArmor profile, which `ci/os-services.sh` loads; a pod keeps its
/// service variable.
fn fixed(environment: &str) -> fn(&mut Command) {
    match environment {
        "kubernetes_runtime_default" => |command| {
            command.env("KUBERNETES_SERVICE_HOST", "10.96.0.1");
        },
        _ => |_| {},
    }
}

/// On each host where a fix exists, the run's first page is refused with
/// `browser_sandbox_unavailable`, not retryable, naming the environment and
/// a fix for it, and no browser starts; the same run on the host with the
/// fix applied opens its page in a sandboxed browser (positive control).
/// Covers: story:4-86, error:browser_sandbox_unavailable, operation:run.new_page, setting:security.browser_sandbox
#[rstest]
#[case::dockers_default_seccomp("docker_default_seccomp", "--security-opt seccomp=")]
#[case::kubernetes_runtime_default("kubernetes_runtime_default", "seccompProfile")]
#[case::apparmors_restriction("apparmor_userns", "userns")]
fn a_host_that_refuses_the_sandbox_where_a_fix_exists_refuses_browser_runs_naming_it(
    #[case] environment: &str,
    #[case] fix_names: &str,
) {
    if environment == "apparmor_userns" {
        assert!(
            apparmor_restricts_user_namespaces(),
            "this Linux host does not restrict unprivileged user namespaces with AppArmor, which the case needs (Ubuntu 23.10 and later, with ci/os-services.sh setup)"
        );
    }
    let (refused_env, fixed_env) = (TestEnv::new(), TestEnv::new());

    let refused = open_a_page(&refused_env, "refused", host(environment));
    let passed = open_a_page(&fixed_env, "fixed", fixed(environment));

    let error = &refused.opened["error"]["data"];
    assert_eq!(
        (
            &error["code"],
            &error["retryable"],
            &error["environment"],
            error["fix"]
                .as_str()
                .unwrap_or_default()
                .contains(fix_names),
            error["fix"]
                .as_str()
                .unwrap_or_default()
                .contains("`security.browser_sandbox` off in Operator policy"),
            refused.command_lines.len(),
        ),
        (
            &json!("browser_sandbox_unavailable"),
            &json!(false),
            &json!(environment),
            true,
            true,
            0,
        ),
        "{}\n{}",
        refused.opened,
        refused.stderr
    );
    assert_eq!(
        (
            passed.opened.get("result").is_some(),
            passed.command_lines.len(),
            passed
                .command_lines
                .iter()
                .any(|line| line.contains("--no-sandbox")),
        ),
        (true, 1, false),
        "{}\n{}",
        passed.opened,
        passed.stderr
    );
}

/// The switches of `security.browser_sandbox` the Audit log holds, each as
/// its Run, value and layer.
fn sandbox_switches(env: &TestEnv) -> Vec<Value> {
    let exported = cli(env, COLLIE).args(["audit", "export"]).run().unwrap();
    exported
        .stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| {
            entry["kind"] == json!("security_switch_turned_off")
                && entry["details"]["setting"] == json!("security.browser_sandbox")
        })
        .map(|entry| json!([entry["run_id"], entry["details"]["value"], entry["layer"]]))
        .collect()
}

/// The engine's warnings that name the sandbox's switch.
fn sandbox_warnings(stderr: &str) -> usize {
    stderr
        .lines()
        .filter(|line| line.contains("WARN") && line.contains("security.browser_sandbox"))
        .count()
}

/// On a platform that allows no sandbox, which AWS Lambda names with its
/// variable, the run proceeds with the sandbox off: its browser starts with
/// `--no-sandbox`, and the switch is reported once, as one Audit log entry,
/// in `weakened` and as one warning.
/// Covers: story:4-86, setting:security.browser_sandbox, operation:run.new_page, command:audit.export
#[test]
fn on_a_platform_that_allows_no_sandbox_the_run_proceeds_with_it_off_and_reported() {
    let env = TestEnv::new();

    let seen = open_a_page(&env, "lambda", |command| {
        under_default_seccomp(command);
        command.env("AWS_LAMBDA_FUNCTION_NAME", "shop");
    });

    assert_eq!(
        (
            seen.opened.get("result").is_some(),
            seen.command_lines
                .iter()
                .map(|line| line.contains("--no-sandbox"))
                .collect::<Vec<_>>(),
            &seen.report["weakened"],
            sandbox_switches(&env),
            sandbox_warnings(&seen.stderr),
        ),
        (
            true,
            vec![true],
            &json!([{ "key": "security.browser_sandbox", "value": false, "layer": "default" }]),
            vec![json!(["lambda", false, "default"])],
            1,
        ),
        "{}\n{}",
        seen.opened,
        seen.stderr
    );
}

/// Writes an Operator policy that switches the sandbox off.
fn switching_the_sandbox_off(env: &TestEnv) {
    let layout = HomeLayout::of(env.home());
    std::fs::create_dir_all(layout.per_major()).unwrap();
    std::fs::write(
        layout.operator_policy(),
        "version = \"0.1\"\n\n[settings.security]\nbrowser_sandbox = false\n",
    )
    .unwrap();
}

/// An Operator policy switches the sandbox off: the run's browser starts
/// with `--no-sandbox`, and the switch is audited and reported with the
/// policy's layer. A Client that switches it off itself, with no such
/// policy, is refused with `setting_locked` naming where it may be set;
/// the same run with every control on starts (positive control).
/// Covers: story:4-87, setting:security.browser_sandbox, error:setting_locked, operation:run.start, operation:run.new_page, command:audit.export
#[test]
fn an_operator_policy_switches_the_sandbox_off_audited_and_a_clients_own_switch_is_locked() {
    let (policy_env, client_env) = (TestEnv::new(), TestEnv::new());
    switching_the_sandbox_off(&policy_env);

    let seen = open_a_page(&policy_env, "policy", |_| {});
    let mut engine = Attached::start(&client_env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    let switched = engine.ask(
        1,
        "run.start",
        &json!({ "run_id": "client", "settings": { "security": { "browser_sandbox": false } } }),
    );
    let started = engine.ask(2, "run.start", &json!({ "run_id": "on" }));
    let _ = engine.finish();

    assert_eq!(
        (
            seen.command_lines
                .iter()
                .map(|line| line.contains("--no-sandbox"))
                .collect::<Vec<_>>(),
            &seen.report["weakened"],
            sandbox_switches(&policy_env),
            sandbox_warnings(&seen.stderr),
        ),
        (
            vec![true],
            &json!([{ "key": "security.browser_sandbox", "value": false, "layer": "operator_policy" }]),
            vec![json!(["policy", false, "operator_policy"])],
            1,
        ),
        "{}\n{}",
        seen.opened,
        seen.stderr
    );
    assert_eq!(
        (
            &switched["error"]["data"]["code"],
            &switched["error"]["data"]["key"],
            started.get("result").is_some(),
            sandbox_switches(&client_env),
        ),
        (
            &json!("setting_locked"),
            &json!("security.browser_sandbox"),
            true,
            vec![]
        ),
        "{switched}"
    );
}

/// The `/proc/<pid>/status` fields of the process `pid` named `names`.
fn status_of(pid: u32, names: &[&str]) -> Vec<String> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    names
        .iter()
        .map(|name| {
            status
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{name}:")))
                .map_or_else(String::new, |value| format!("{name}: {}", value.trim()))
        })
        .collect()
}

/// The process IDs of the test's browsers whose command line `matches`.
fn browser_processes(env: &TestEnv, matches: impl Fn(&str) -> bool) -> Vec<u32> {
    let name = env.folder().file_name().unwrap().to_string_lossy();
    let marker = name.rsplit(' ').next().unwrap().to_owned();
    let listed = Command::new("ps")
        .args(["-axww", "-o", "pid=,args="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter(|line| line.contains(&marker) && line.contains("--user-data-dir="))
        .filter(|line| matches(line))
        .filter_map(|line| line.split_whitespace().next()?.parse().ok())
        .collect()
}

fn user_namespace_of(pid: u32) -> String {
    std::fs::read_link(format!("/proc/{pid}/ns/user"))
        .map(|link| link.display().to_string())
        .unwrap_or_default()
}

/// Every renderer of a launched Chromium is sandboxed: on Linux, under a
/// seccomp filter with `no_new_privs` (TESTING.md, Confinement), in a user
/// namespace other than its browser's, and the browser was started without
/// `--no-sandbox`.
/// Covers: story:4-85, setting:security.browser_sandbox, operation:run.new_page, operation:page.goto
#[test]
fn every_renderer_of_a_launched_chromium_is_sandboxed() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "sandboxed",
        &Map::new(),
        |_| {},
    );
    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "sandboxed" })));
    let went = engine.ask(
        3,
        "page.goto",
        &json!({
            "page_id": page.page_id,
            "url": proving_ground.url("catalog.test", "/catalog/1"),
        }),
    );

    let browsers = browser_processes(&env, |line| !line.contains("--type="));
    let renderers = browser_processes(&env, |line| line.contains("--type=renderer"));
    let command_lines = browser_command_lines(&env);
    let browser_namespace = browsers.first().map(|pid| user_namespace_of(*pid));
    // A renderer that exited since it was listed, as the blank page's does
    // once the navigation gives the site its own, has no status to read.
    let confinement: BTreeSet<Vec<String>> = renderers
        .iter()
        .map(|pid| (*pid, status_of(*pid, &["Seccomp", "NoNewPrivs"])))
        .filter(|(pid, status)| {
            std::path::Path::new(&format!("/proc/{pid}")).exists()
                || status.iter().any(|field| !field.is_empty())
        })
        .map(|(_, status)| status)
        .collect();
    let own_namespaces = renderers
        .iter()
        .all(|pid| Some(user_namespace_of(*pid)) != browser_namespace);
    engine.ask(4, "run.finish", &json!({ "run_id": "sandboxed" }));
    let _ = engine.finish();

    assert!(went.get("result").is_some(), "{went}");
    assert_eq!(
        (
            browsers.len(),
            !renderers.is_empty(),
            confinement,
            own_namespaces,
            command_lines
                .iter()
                .any(|line| line.contains("--no-sandbox")),
        ),
        (
            1,
            true,
            BTreeSet::from([vec!["Seccomp: 2".to_owned(), "NoNewPrivs: 1".to_owned()]]),
            true,
            false,
        ),
        "{command_lines:#?}"
    );
}
