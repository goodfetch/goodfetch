//! What a crawl's start from a Client is checked for before its Run
//! begins (ADRs 0036, 0078, 0102 and 0103): a Schedule's reserved tag is
//! refused, a `detached` start of a crawl the program serves, or on an
//! engine that ends with its Connection, fails with `detach_unsupported`,
//! and a keyed Sink with no key for its Datasets with `setting_invalid`.

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use super::{Reply, crawl, dependencies, seeds};

/// A code-free crawl's `extraction`, which the engine reads each page with.
fn extraction() -> Value {
    json!({ "formats": { "markdown": true } })
}

/// Sends `crawl.start` with `start` for the Run `refused`, and answers the
/// reply and whether that Run's folder exists.
fn started(start: Value) -> (Value, bool) {
    let env = TestEnv::new();
    let mut params = json!({
        "run_id": "refused",
        "seeds": seeds(1),
        "labels": { "page": {} },
        "datasets": [],
    });
    let Value::Object(start) = start else {
        panic!("a start is an object");
    };
    for (key, value) in start {
        params[key] = value;
    }

    let reply = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call("crawl.start", params, &mut |_| Reply::Silent)
                .await
        },
    );

    let begun = env.home().join("1").join("runs").join("refused").exists();
    (reply, begun)
}

/// A Sink in a keyed write mode is refused with `setting_invalid` naming
/// its `key`, before its Run begins, when it names no key and its Dataset
/// declares none, or when its key names a field its Dataset's schema does
/// not require (ADR 0103).
///
/// Covers: story:6-286, error:setting_invalid
#[rstest]
#[case::no_key(json!({ "name": "products" }), json!(null))]
#[case::a_field_not_required(
    json!({
        "name": "products",
        "schema": {
            "type": "object",
            "properties": { "sku": { "type": "string" }, "price": { "type": "number" } },
            "required": ["sku"],
        },
        "key": ["sku"],
    }),
    json!(["price"])
)]
fn a_keyed_sink_without_a_key_its_datasets_require_is_setting_invalid(
    #[case] dataset: Value,
    #[case] key: Value,
) {
    let mut sink = json!({
        "type": "sqlite",
        "datasets": ["products"],
        "mode": "upsert",
        "destination": "env:WAREHOUSE",
    });
    if !key.is_null() {
        sink["key"] = key;
    }

    let (refused, begun) = started(json!({
        "datasets": [dataset],
        "settings": { "sinks": { "warehouse": sink } },
    }));

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            &refused["error"]["data"]["key"],
            begun,
        ),
        (
            &json!("setting_invalid"),
            &json!("sinks.warehouse.key"),
            false
        ),
        "{refused}"
    );
}

/// A crawl's start carrying a Schedule's reserved tag is refused with
/// `invalid_params` naming the tag, before its Run begins.
///
/// Covers: story:3-173, operation:crawl.start
#[test]
fn a_crawl_start_with_a_schedules_reserved_tag_is_invalid_params() {
    let (refused, begun) = started(json!({ "tags": ["goodfetch.schedule:nightly"] }));

    assert_eq!(
        (&refused["error"]["data"]["code"], begun),
        (&json!("invalid_params"), false),
        "{refused}"
    );
}

/// A detached crawl must need nothing of the program: one whose requests
/// go to the program's handler, with no `extraction`, or whose Hooks run in
/// the program fails with `detach_unsupported` naming that key, in its
/// `fix` too; a code-free crawl with no Hook passes that check and is
/// refused only because this Attached engine ends with its Connection,
/// its `fix` naming `engine: daemon`.
///
/// Covers: story:3-174, error:detach_unsupported
#[rstest]
#[case::handler(json!({ "detached": true }), "extraction", "`extraction`")]
#[case::hooks(
    json!({
        "detached": true,
        "extraction": extraction(),
        "hooks": [{ "name": "sign", "stage": "before_record_commit" }],
    }),
    "hooks",
    "`hooks`"
)]
#[case::request_stage_hooks(
    json!({
        "detached": true,
        "extraction": extraction(),
        "hooks": [{ "name": "sign", "stage": "before_request" }],
    }),
    "hooks",
    "`hooks`"
)]
#[case::attached(
    json!({ "detached": true, "extraction": extraction() }),
    "engine",
    "`engine: daemon`"
)]
fn a_detached_crawl_start_the_run_cannot_outlive_is_detach_unsupported_naming_why(
    #[case] start: Value,
    #[case] key: &str,
    #[case] fix_names: &str,
) {
    let (refused, begun) = started(start);

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            &refused["error"]["data"]["key"],
            refused["error"]["data"]["fix"]
                .as_str()
                .is_some_and(|fix| fix.contains(fix_names)),
            begun,
        ),
        (&json!("detach_unsupported"), &json!(key), true, false),
        "{refused}"
    );
}
