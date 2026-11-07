//! The switchable security controls through Collie's executable (spec
//! #564's seam; ADRs 0016, 0028 and 0102): one `security.*` setting per
//! control that costs speed, each on by default and settable from the run
//! up, which `config explain security` lists with the layer that set it;
//! an Operator policy that locks one refuses a run's attempt to switch it
//! off with `setting_locked`, and where nothing locks it the run switches
//! it off.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the Goodfetch home's Operator policy themselves"
)]

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{Mode, TestEnv, cli};
use serde_json::{Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// `config explain security` lists the seven controls of both Products, each
/// on by default, and the one the user configuration switched off with that
/// layer, marked as weakening.
/// Covers: story:4-74, command:config.explain, setting:security.field_encryption, setting:security.secure_delete, setting:security.resolved_ip_check, setting:security.raw_cdp_egress_proxy, setting:security.browser_sandbox, setting:security.audit_chain, setting:security.redaction
#[test]
fn config_explain_security_lists_every_switchable_control_with_the_layer_that_set_it() {
    let env = TestEnv::new();
    env.user_configuration("security.redaction = false\n");

    let printed = cli(&env, COLLIE)
        .args(["config", "explain", "security", "--json"])
        .run()
        .unwrap();

    let explained: Value = serde_json::from_str(&printed.stdout).unwrap();
    let keys: Vec<Value> = explained["settings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|setting| {
            json!([
                setting["key"],
                setting["value"],
                setting["layer"],
                setting["weakens"],
                setting["products"]
            ])
        })
        .collect();
    assert_eq!(
        keys,
        [
            json!([
                "security.audit_chain",
                true,
                "default",
                false,
                ["collie", "foxhound"]
            ]),
            json!([
                "security.browser_sandbox",
                true,
                "default",
                false,
                ["collie", "foxhound"]
            ]),
            json!([
                "security.field_encryption",
                true,
                "default",
                false,
                ["collie", "foxhound"]
            ]),
            json!([
                "security.raw_cdp_egress_proxy",
                true,
                "default",
                false,
                ["collie", "foxhound"]
            ]),
            json!([
                "security.redaction",
                false,
                "user_configuration",
                true,
                ["collie", "foxhound"]
            ]),
            json!([
                "security.resolved_ip_check",
                true,
                "default",
                false,
                ["collie", "foxhound"]
            ]),
            json!([
                "security.secure_delete",
                true,
                "default",
                false,
                ["collie", "foxhound"]
            ]),
        ]
    );
}

/// Writes an Operator policy that locks `security.audit_chain` on.
fn locking_the_audit_chain(env: &TestEnv) {
    let layout = HomeLayout::of(env.home());
    std::fs::create_dir_all(layout.per_major()).unwrap();
    std::fs::write(
        layout.operator_policy(),
        "version = \"0.1\"\n\n[lock.security]\naudit_chain = true\n",
    )
    .unwrap();
}

/// What `run start` with the Audit log's chain switched off answered.
fn started_unchained(env: &TestEnv) -> Value {
    let checked = cli(env, COLLIE)
        .args(["run", "start", "--run-id", "unchained", "--json"])
        .args(["--set", "security.audit_chain=false"])
        .mode(Mode::Attached)
        .run()
        .unwrap();
    serde_json::from_str(&checked.stdout).unwrap()
}

/// An Operator policy that locks the switch on refuses the run that
/// switches it off with `setting_locked`, naming the lock; on a home
/// whose policy does not lock it, the same run starts (positive control).
/// Covers: story:4-75, error:setting_locked, setting:security.audit_chain, command:run.start
#[test]
fn an_operator_policy_that_locks_a_switch_refuses_a_run_that_switches_it_off() {
    let (locked, unlocked) = (TestEnv::new(), TestEnv::new());
    locking_the_audit_chain(&locked);

    let (refused, started) = (started_unchained(&locked), started_unchained(&unlocked));

    assert_eq!(
        (
            &refused["error"]["code"],
            &refused["error"]["limit"],
            &started["lifecycle"],
        ),
        (
            &json!("setting_locked"),
            &json!({ "key": "security.audit_chain", "value": true, "layer": "operator_policy" }),
            &json!("running"),
        ),
        "{refused} {started}"
    );
}
