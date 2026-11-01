//! `needs-you` through Collie's executable (spec #592; ADR 0069): `needs-you
//! list` prints what waits for a person, nothing on a fresh home with the
//! storage levels off, and `needs-you dismiss` of an item the list does
//! not hold dismisses nothing.

use goodfetch_test_support::{TestEnv, cli};
use serde_json::{Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Covers: command:needs-you.list, command:needs-you.dismiss
#[test]
fn needs_you_lists_nothing_on_a_fresh_home_and_dismisses_no_item_it_does_not_hold() {
    let env = TestEnv::new();
    // Both levels off: no filesystem of the machine is listed, however full.
    env.user_configuration("storage.low_free = 0\nstorage.critical_free = 0\n");

    let listed = cli(&env, COLLIE)
        .args(["needs-you", "list", "--json"])
        .run()
        .unwrap();
    let dismissed = cli(&env, COLLIE)
        .args(["needs-you", "dismiss", "--item", "gone", "--json"])
        .run()
        .unwrap();

    let parsed = |stdout: &str| serde_json::from_str::<Value>(stdout).unwrap_or(Value::Null);
    assert_eq!(
        (
            listed.exit,
            parsed(&listed.stdout),
            dismissed.exit,
            parsed(&dismissed.stdout)
        ),
        (
            Some(0),
            json!({ "items": [] }),
            Some(0),
            json!({ "dismissed": false })
        ),
        "{listed}\n{dismissed}"
    );
}
