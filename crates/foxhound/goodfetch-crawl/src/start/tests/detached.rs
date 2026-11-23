//! What a crawl's start from a Client is checked for before its Run
//! begins (ADRs 0036, 0078 and 0102): a Schedule's reserved tag is refused,
//! and a `detached` start of a crawl the program serves, or on an engine
//! that ends with its Connection, fails with `detach_unsupported`.

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

/// A crawl's start carrying a Schedule's reserved tag is refused with
/// `invalid_params` naming the tag, before its Run begins.
///
/// Covers: story:3-173, operation:crawl.start
#[test]
fn a_crawl_start_with_a_schedules_reserved_tag_is_invalid_params() {
    let (refused, begun) = started(json!({ "tags": ["goodfetch.schedule:nightly"] }));

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            refused["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("goodfetch.schedule:nightly")),
            begun,
        ),
        (&json!("invalid_params"), true, false),
        "{refused}"
    );
}

/// A detached crawl must need nothing of the program: one whose requests
/// go to the program's handler, with no `extraction`, or whose Hooks run in
/// the program fails with `detach_unsupported` naming that key; a code-free
/// crawl with no Hook passes that check and is refused only because this
/// Attached engine ends with its Connection, naming `engine`.
///
/// Covers: story:3-174, error:detach_unsupported
#[rstest]
#[case::handler(json!({ "detached": true }), "extraction")]
#[case::hooks(
    json!({
        "detached": true,
        "extraction": extraction(),
        "hooks": [{ "name": "sign", "stage": "before_record_commit" }],
    }),
    "hooks"
)]
#[case::attached(json!({ "detached": true, "extraction": extraction() }), "engine")]
fn a_detached_crawl_start_the_run_cannot_outlive_is_detach_unsupported_naming_why(
    #[case] start: Value,
    #[case] key: &str,
) {
    let (refused, begun) = started(start);

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            &refused["error"]["data"]["key"],
            begun,
        ),
        (&json!("detach_unsupported"), &json!(key), false),
        "{refused}"
    );
}
