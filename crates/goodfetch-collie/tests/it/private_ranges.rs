//! `egress.private_ranges` and `security.resolved_ip_check` on Chromium
//! over stdio (spec #563, stories 4-11 and 4-17; ADRs 0016 and 0017): a
//! page reaches a Proving ground host on loopback through the egress relay
//! only where the Run lists its range, which the Audit log records once as
//! `private_addresses_allowed` and the Run report lists in `weakened`; with
//! the resolved-address check switched off the same host is reached with
//! no range listed, the switch recorded, reported and warned once, and an
//! Operator policy that locks the check on refuses the switch. The Proving
//! ground's request record is the oracle: a refused page reaches no site.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the Goodfetch home's Operator policy themselves"
)]

use goodfetch_protocol::catalog::Product;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache, cli};
use serde_json::{Map, Value, json};

use crate::page::{opened, started_with};
use goodfetch_test_client::Attached;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The Proving ground's own seed for these tests.
const SEED: u64 = 622;

/// What a Run of `settings` met: its page's answer to a `goto` to the
/// Proving ground's `catalog.test`, how many of the Proving ground's
/// requests reached that page, the Run report `run.finish` answered and
/// what the engine wrote on stderr.
struct Met {
    went: Value,
    reached: usize,
    report: Value,
    stderr: String,
}

fn ran(env: &TestEnv, proving_ground: &ProvingGround, run_id: &str, settings: &Value) -> Met {
    let cache = browser_cache(env).unwrap();
    let settings: Map<String, Value> = settings.as_object().cloned().unwrap_or_default();
    let mut engine = started_with(env, &cache, proving_ground, run_id, &settings, |_| {});
    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": run_id })))
        .page_id
        .0;
    let target = format!("/catalog/1?run={run_id}");
    let went = engine.ask(
        3,
        "page.goto",
        &json!({ "page_id": page, "url": proving_ground.url("catalog.test", &target) }),
    );
    let reached = proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request["target"] == json!(target))
        .count();
    let report = engine.ask(4, "run.finish", &json!({ "run_id": run_id }))["result"].clone();
    let (_, stderr) = engine.finish_with_stderr();
    Met {
        went,
        reached,
        report,
        stderr,
    }
}

/// The Audit log's entries of `kind` for the Run `run_id`, each its
/// details.
fn audited(env: &TestEnv, run_id: &str, kind: &str) -> Vec<Value> {
    cli(env, COLLIE)
        .args(["audit", "export"])
        .run()
        .unwrap()
        .stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["run_id"] == json!(run_id) && entry["kind"] == json!(kind))
        .map(|entry| entry["details"].clone())
        .collect()
}

/// The keys of a Run report's `weakened`.
fn weakened(report: &Value) -> Vec<Value> {
    report["weakened"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|weakened| weakened["key"].clone())
        .collect()
}

/// A Proving ground host on 127.0.0.0/8, reached through the Run's
/// `network.resolve` mapping and the egress relay, loads where the Run
/// lists the range and is refused naming `egress.private_ranges` where it
/// lists none (positive control): the listing Run alone writes one
/// `private_addresses_allowed` entry naming its ranges, and its Run report
/// lists the setting in `weakened`.
///
/// Covers: story:4-11, story:4-13, error:egress_refused, setting:egress.private_ranges, operation:page.goto, operation:run.finish
#[test]
fn a_loopback_host_loads_only_where_its_range_is_listed_and_the_listing_is_recorded() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();

    let listed = ran(
        &env,
        &proving_ground,
        "listed",
        &json!({ "egress": { "private_ranges": ["127.0.0.0/8"] } }),
    );
    let unlisted = ran(
        &env,
        &proving_ground,
        "unlisted",
        &json!({ "egress": { "private_ranges": [] } }),
    );

    assert_eq!(
        (
            listed.went["result"]["response"]["status"].clone(),
            listed.reached
        ),
        (json!(200), 1),
        "{}",
        listed.went
    );
    let refused = &unlisted.went["error"]["data"];
    assert_eq!(
        (
            &refused["code"],
            &refused["rule"],
            &refused["setting"],
            unlisted.reached,
        ),
        (
            &json!("egress_refused"),
            &json!("private_address"),
            &json!("egress.private_ranges"),
            0
        ),
        "{}",
        unlisted.went
    );
    assert_eq!(
        (
            audited(&env, "listed", "private_addresses_allowed"),
            audited(&env, "unlisted", "private_addresses_allowed"),
        ),
        (
            vec![json!({ "setting": "egress.private_ranges", "ranges": ["127.0.0.0/8"] })],
            vec![]
        )
    );
    assert_eq!(
        (weakened(&listed.report), weakened(&unlisted.report)),
        (vec![json!("egress.private_ranges")], vec![]),
        "{} {}",
        listed.report,
        unlisted.report
    );
}

/// With `security.resolved_ip_check` off and no range listed, the same
/// loopback host loads: the switch is recorded once in the Audit log, the
/// Run report lists it in `weakened`, and the engine warns once naming it.
///
/// Covers: story:4-17, story:4-74, setting:security.resolved_ip_check, operation:page.goto, operation:run.finish
#[test]
fn with_the_resolved_address_check_off_the_loopback_host_loads_and_the_switch_is_reported() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();

    let unchecked = ran(
        &env,
        &proving_ground,
        "unchecked",
        &json!({
            "egress": { "private_ranges": [] },
            "security": { "resolved_ip_check": false },
        }),
    );

    let warnings = unchecked
        .stderr
        .lines()
        .filter(|line| line.contains("WARN") && line.contains("security.resolved_ip_check"))
        .count();
    assert_eq!(
        (
            unchecked.went["result"]["response"]["status"].clone(),
            unchecked.reached,
            audited(&env, "unchecked", "security_switch_turned_off"),
            weakened(&unchecked.report),
            warnings,
        ),
        (
            json!(200),
            1,
            vec![json!({ "setting": "security.resolved_ip_check", "value": false })],
            vec![json!("security.resolved_ip_check")],
            1
        ),
        "{}",
        unchecked.went
    );
}

/// An Operator policy that locks `security.resolved_ip_check` on refuses
/// the Run that switches it off with `setting_locked` naming the lock.
///
/// Covers: story:4-17, error:setting_locked, setting:security.resolved_ip_check, operation:run.start
#[test]
fn an_operator_policy_that_locks_the_check_refuses_the_switch() {
    let env = TestEnv::new();
    let layout = HomeLayout::of(env.home());
    std::fs::create_dir_all(layout.per_major()).unwrap();
    std::fs::write(
        layout.operator_policy(),
        "version = \"0.1\"\n\n[lock.security]\nresolved_ip_check = true\n",
    )
    .unwrap();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&goodfetch_test_client::hello(Product::Collie));
    let _ = engine.next();

    let refused = engine.ask(
        1,
        "run.start",
        &json!({ "run_id": "locked", "settings": { "security": { "resolved_ip_check": false } } }),
    );
    let _ = engine.finish();

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            &refused["error"]["data"]["limit"]
        ),
        (
            &json!("setting_locked"),
            &json!({ "key": "security.resolved_ip_check", "value": true, "layer": "operator_policy" })
        ),
        "{refused}"
    );
}

/// A Client acting for a model never sets `egress.private_ranges`: its
/// `run.start` that lists one is refused with `setting_locked`, and the
/// program's own `run.start` with the same value starts (positive
/// control).
///
/// Covers: story:4-11, error:setting_locked, setting:egress.private_ranges, operation:run.start
#[test]
fn a_client_acting_for_a_model_lists_no_private_range() {
    let env = TestEnv::new();
    let start = |surface: Option<&str>| {
        let mut engine = Attached::start(&env, COLLIE);
        let mut hello = goodfetch_test_client::hello(Product::Collie);
        if let Some(surface) = surface {
            hello["params"]["surface"] = json!(surface);
        }
        engine.send(&hello);
        let _ = engine.next();
        let answered = engine.ask(
            1,
            "run.start",
            &json!({
                "run_id": surface.unwrap_or("program"),
                "settings": { "egress": { "private_ranges": ["127.0.0.0/8"] } },
            }),
        );
        let _ = engine.finish();
        answered
    };

    let (for_a_model, own) = (start(Some("mcp")), start(None));

    assert_eq!(
        (
            &for_a_model["error"]["data"]["code"],
            &for_a_model["error"]["data"]["key"],
            &own["result"]["lifecycle"],
        ),
        (
            &json!("setting_locked"),
            &json!("egress.private_ranges"),
            &json!("running")
        ),
        "{for_a_model} {own}"
    );
}
