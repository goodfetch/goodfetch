//! `usage` through Collie's executable (spec #213's Seam 1; ADRs 0041,
//! 0059 and 0090): `usage set` adds a Limit to the user's Usage ledger, in
//! the Goodfetch home `GOODFETCH_HOME` names, outside every per-major
//! folder; `usage list` shows the Usage against each Limit by scope; and a
//! flag naming a value only Foxhound carries is refused with
//! `not_in_product`.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{TestEnv, cli};
use serde_json::{Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Covers: story:3-67, story:3-79, command:usage.set, command:usage.list
#[test]
fn usage_lists_a_limit_set_in_the_ledger_of_the_goodfetch_home() {
    let env = TestEnv::new();
    let set = cli(&env, COLLIE)
        .args([
            "usage",
            "set",
            "--scope",
            "install",
            "--usage-kind",
            "requests",
            "--amount",
            "500",
        ])
        .run()
        .unwrap();
    assert_eq!(set.exit, Some(0), "{set}");

    let listed = cli(&env, COLLIE)
        .args(["usage", "list", "--json"])
        .run()
        .unwrap();

    let listed: Value = serde_json::from_str(&listed.stdout).unwrap();
    assert_eq!(
        (listed, env.home().join("usage-ledger.sqlite").is_file()),
        (
            json!({ "limits": [{
                "limit": {
                    "scope": { "kind": "install", "id": "" },
                    "usage_kind": "requests",
                    "amount": 500,
                    "must_understand": true,
                    "set_by": "client",
                },
                "standing": "held",
                "charged": 0,
                "held": 0,
                "left": 500,
            }]}),
            true
        )
    );
}

/// Covers: story:3-67, error:not_in_product
#[test]
fn a_flag_naming_paid_fetch_is_refused_by_collie() {
    let env = TestEnv::new();

    let refused = cli(&env, COLLIE)
        .args([
            "usage",
            "set",
            "--scope",
            "install",
            "--usage-kind",
            "paid_fetch",
            "--currency",
            "USD",
            "--amount",
            "5000000",
            "--json",
        ])
        .run()
        .unwrap();

    let printed: Value = serde_json::from_str(&refused.stdout).unwrap();
    assert_eq!(
        (&printed["error"]["code"], refused.exit),
        (&json!("not_in_product"), Some(10))
    );
}
