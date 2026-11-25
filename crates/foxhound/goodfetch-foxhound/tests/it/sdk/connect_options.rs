//! What a Foxhound connection opens with (ADRs 0083, 0088 and 0104;
//! #3310): Collie's options, with Foxhound's settings, and the program's
//! own Fetch backends and Sinks, each registered by name, async and
//! through `blocking`. A key the engine reads only from its configuration,
//! such as `storage.low_free`, has no typed constant, so code cannot set
//! it; the engine still explains it from the file that set it.
//! TypeScript's `connect-options.test.ts` and Python's
//! `test_connect_options.py` are its twins, which also see the engine
//! refuse it in `hello`.

use goodfetch_foxhound::settings::{Settings, crawl};
use goodfetch_foxhound::{ExplainedSetting, Foxhound, SettingsExplanation, blocking};
use goodfetch_test_support::{TestEnv, real_time_runtime};
use serde_json::{Value, json};

use super::options;

/// Each explained setting as its key, value and layer.
fn keyed(explanation: &SettingsExplanation) -> Vec<Value> {
    explanation
        .settings
        .iter()
        .map(
            |ExplainedSetting {
                 key, value, layer, ..
             }| json!([key, value, layer]),
        )
        .collect()
}

/// A connection takes the program's own Fetch backends and Sinks, and
/// Foxhound's settings beside them.
/// Covers: story:11-1008
#[test]
fn a_connection_registers_the_programs_own_fetch_backends_and_sinks_by_name() {
    let env = TestEnv::new();
    let explained = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(
            options(&env)
                .settings(Settings::new().set(crawl::MAX_DEPTH, 2))
                .fetch_backend("empty-page")
                .sink("discard"),
        )
        .await
        .unwrap();
        let explained = foxhound
            .explain_settings()
            .keys(["crawl.max_depth"])
            .await
            .unwrap();
        foxhound.close().await.unwrap();
        explained
    });

    assert_eq!(
        keyed(&explained),
        vec![json!(["crawl.max_depth", 2, "run"])]
    );
}

/// A configuration-only key is explained from the file that set it, on a
/// connection that registered its own handlers, without async.
/// Covers: story:11-1009
#[test]
fn a_configuration_only_key_is_explained_from_its_file() {
    let env = TestEnv::new();
    env.user_configuration("storage.low_free = 1000\n");
    let foxhound =
        blocking::Foxhound::connect(options(&env).fetch_backend("empty-page").sink("discard"))
            .unwrap();

    let explained = foxhound
        .explain_settings()
        .keys(["storage.low_free"])
        .send()
        .unwrap();
    foxhound.close().unwrap();

    assert_eq!(
        keyed(&explained),
        vec![json!(["storage.low_free", 1000, "user_configuration"])]
    );
}
