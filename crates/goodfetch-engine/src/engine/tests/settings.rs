//! A Run's settings through the engine's public interface over the
//! in-memory network (spec #213's Seam 3): `run.start`'s `settings` and the
//! Goodfetch home's Operator policy and user configuration resolved before
//! the Run begins, refused with their Error codes, the run report naming
//! the policy's version and the settings that weaken, `config.explain`,
//! and a project file sent to an Exposed Daemon, held there to the run
//! layer's authority.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the Goodfetch home's settings files themselves"
)]

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use super::daemon::{TOKEN, daemon as served, hello_with, no_handover, options as daemon_options};
use super::run::call;
use super::websocket::exposed;
use super::{attached_on, engine, engine_with, hello};
use crate::daemon::DaemonOptions;

/// Writes `text` as `file` of the test home's per-major folder.
fn write(env: &TestEnv, file: &str, text: &str) {
    let folder = env.home().join("1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(file), text).unwrap();
}

/// Starts a Run with `params` on an engine of `product`, and finishes it
/// when it starts; answers the start's and the finish's replies.
fn start_and_finish(env: &TestEnv, product: Product, params: Value) -> (Value, Value) {
    attached_on(&engine_with(product, None), env, |mut client| async move {
        client.send(&[hello(product.as_str(), "0.1")]).await;
        client.next().await;
        let started = call(&mut client, 1, "run.start", params).await;
        let finished = call(&mut client, 2, "run.finish", json!({ "run_id": "set" })).await;
        (started, finished)
    })
}

/// Covers: story:3-88, error:setting_locked
#[test]
fn a_run_whose_settings_an_operator_policy_locked_is_refused_before_it_begins() {
    let env = TestEnv::new();
    write(
        &env,
        "operator-policy.toml",
        "version = \"0.1\"\n\n[lock.timeout]\nnavigation = 10000\n",
    );

    let (started, _) = start_and_finish(
        &env,
        Product::Collie,
        json!({ "run_id": "set", "settings": { "timeout": { "navigation": 1000 } } }),
    );

    assert_eq!(
        (
            &started["error"]["data"]["code"],
            &started["error"]["data"]["limit"],
            env.home().join("1").join("runs").join("set").exists(),
        ),
        (
            &json!("setting_locked"),
            &json!({ "key": "timeout.navigation", "value": 10000, "layer": "operator_policy" }),
            false,
        )
    );
}

/// A Run that opens with a setting whose feature the engine does not build
/// yet, set away from its default by the run or by configuration, is
/// refused before it begins with `capability_unavailable` naming the key
/// and the layer that set it: the value would otherwise be ignored (ADR
/// 0083).
///
/// Covers: story:3-88, error:capability_unavailable
#[rstest]
/// Covers: setting:wall.domains
#[case::wall_domains_in_configuration(
    "wall.domains = [\"paper.example\"]\n",
    json!({}),
    "wall.domains",
    "user_configuration"
)]
/// Covers: setting:security.secure_delete
#[case::secure_delete_switched_off(
    "",
    json!({ "security": { "secure_delete": false } }),
    "security.secure_delete",
    "run"
)]
/// Covers: setting:security.field_encryption
#[case::field_encryption_switched_off(
    "",
    json!({ "security": { "field_encryption": false } }),
    "security.field_encryption",
    "run"
)]
/// Covers: setting:sample.echo_repeats
#[case::the_sample_settings_repeats(
    "",
    json!({ "sample": { "echo_repeats": 2 } }),
    "sample.echo_repeats",
    "run"
)]
fn a_run_setting_what_the_engine_does_not_build_yet_is_refused_before_it_begins(
    #[case] configuration: &str,
    #[case] settings: Value,
    #[case] key: &str,
    #[case] layer: &str,
) {
    let env = TestEnv::new();
    write(
        &env,
        "configuration.toml",
        &format!("version = \"0.1\"\n\n[settings]\n{configuration}"),
    );

    let (started, _) = start_and_finish(
        &env,
        Product::Collie,
        json!({ "run_id": "set", "settings": settings }),
    );

    assert_eq!(
        (
            &started["error"]["data"]["code"],
            &started["error"]["data"]["key"],
            &started["error"]["data"]["layer"],
            env.home().join("1").join("runs").join("set").exists(),
        ),
        (
            &json!("capability_unavailable"),
            &json!(key),
            &json!(layer),
            false
        )
    );
}

/// A setting whose feature is not built, left at its default or set to it,
/// refuses nothing: the Run starts, and its report lists nothing weakened.
///
/// Covers: story:3-88, setting:wall.domains, setting:security.secure_delete, setting:security.field_encryption
#[rstest]
#[case::left_alone(json!({}))]
#[case::set_to_their_defaults(json!({
    "wall": { "domains": [] },
    "security": { "secure_delete": true, "field_encryption": true },
}))]
fn a_run_leaving_an_unbuilt_setting_at_its_default_starts(#[case] settings: Value) {
    let env = TestEnv::new();

    let (started, finished) = start_and_finish(
        &env,
        Product::Collie,
        json!({ "run_id": "set", "settings": settings }),
    );

    assert_eq!(
        (
            &started["result"]["run_id"],
            &finished["result"]["weakened"]
        ),
        (&json!("set"), &Value::Null)
    );
}

/// Covers: story:3-89, story:3-92, setting:tls.accept_invalid_certs
#[test]
fn a_run_report_names_the_operator_policy_version_and_the_settings_that_weaken() {
    let env = TestEnv::new();
    write(
        &env,
        "operator-policy.toml",
        "version = \"0.1\"\n\n[lock.timeout]\nnavigation = 10000\n",
    );

    let (_, finished) = start_and_finish(
        &env,
        Product::Collie,
        json!({
            "run_id": "set",
            "settings": { "tls": { "accept_invalid_certs": { "intranet.example": null } } },
        }),
    );

    assert_eq!(
        (
            &finished["result"]["policy_version"],
            &finished["result"]["weakened"]
        ),
        (
            &json!("sha256:e0e1ddb8a44ecebb"),
            &json!([{
                "key": "tls.accept_invalid_certs",
                "value": { "intranet.example": null },
                "layer": "run",
                "origin": "settings",
            }])
        )
    );
}

/// Covers: story:3-142, error:not_in_product
#[test]
fn a_foxhound_key_in_a_runs_settings_is_not_in_product_on_a_collie_engine() {
    let env = TestEnv::new();

    let (started, _) = start_and_finish(
        &env,
        Product::Collie,
        json!({ "run_id": "set", "settings": { "crawl": { "retry": { "max": 5 } } } }),
    );

    assert_eq!(
        (
            &started["error"]["data"]["code"],
            &started["error"]["data"]["requested"],
            &started["error"]["data"]["needs"]
        ),
        (
            &json!("not_in_product"),
            &json!("crawl.retry.max"),
            &json!("foxhound")
        )
    );
}

/// Covers: story:3-143, error:setting_not_applicable
#[test]
fn a_crawls_key_on_a_collie_run_of_a_foxhound_engine_is_not_applicable() {
    let env = TestEnv::new();

    let (started, _) = start_and_finish(
        &env,
        Product::Foxhound,
        json!({ "run_id": "set", "settings": { "crawl": { "retry": { "max": 5 } } } }),
    );

    assert_eq!(
        started["error"]["data"]["code"],
        json!("setting_not_applicable")
    );
}

/// Covers: story:3-136, error:setting_not_applicable, setting:browser_cache.max_unused_age
#[test]
fn browser_cache_max_unused_age_in_a_runs_settings_is_not_applicable() {
    let env = TestEnv::new();

    let (started, _) = start_and_finish(
        &env,
        Product::Collie,
        json!({ "run_id": "set", "settings": { "browser_cache": { "max_unused_age": 1000 } } }),
    );

    let folder = env.home().join("1");
    assert_eq!(
        (
            &started["error"]["data"]["code"],
            &started["error"]["data"]["layer"],
            &started["error"]["data"]["fix"],
        ),
        (
            &json!("setting_not_applicable"),
            &json!("run"),
            &json!(format!(
                "set `browser_cache.max_unused_age` in user configuration, `{}`, or an Operator policy, `{}`",
                folder.join("configuration.toml").display(),
                folder.join("operator-policy.toml").display()
            )),
        )
    );
}

/// Covers: story:3-86, error:setting_unknown
#[test]
fn a_user_configuration_that_does_not_read_refuses_the_run_with_its_error_code() {
    let env = TestEnv::new();
    write(
        &env,
        "configuration.toml",
        "version = \"0.1\"\n\n[settings.timeout]\nnavigation_ = 1000\n",
    );

    let (started, _) = start_and_finish(&env, Product::Collie, json!({ "run_id": "set" }));

    assert_eq!(
        (
            &started["error"]["data"]["code"],
            &started["error"]["data"]["closest"]
        ),
        (&json!("setting_unknown"), &json!("timeout.navigation"))
    );
}

/// Covers: story:3-89, story:3-142, operation:config.explain, setting:timeout.first_byte
#[test]
fn config_explain_shows_each_value_with_the_layer_that_set_it_and_the_layer_that_locked_it() {
    let env = TestEnv::new();
    write(
        &env,
        "operator-policy.toml",
        "version = \"0.1\"\n\n[lock.timeout]\nnavigation = 10000\n",
    );
    write(
        &env,
        "configuration.toml",
        "version = \"0.1\"\n\n[settings]\ntimeout.first_byte = 5000\ncrawl.retry.max = 4\n",
    );

    let explained = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(
                &mut client,
                1,
                "config.explain",
                json!({ "keys": ["timeout.navigation", "timeout.first_byte", "crawl"] }),
            )
            .await
        },
    );

    assert_eq!(
        explained["result"]["settings"],
        json!([
            {
                "key": "crawl.retry.max",
                "products": ["foxhound"],
                "applied": false,
                "value": 4,
                "layer": "user_configuration",
                "weakens": false,
            },
            {
                "key": "timeout.first_byte",
                "products": ["collie", "foxhound"],
                "applied": true,
                "value": 5000,
                "layer": "user_configuration",
                "weakens": false,
                "user_settable": true,
            },
            {
                "key": "timeout.navigation",
                "products": ["collie", "foxhound"],
                "applied": true,
                "value": 10000,
                "layer": "operator_policy",
                "locked_by": "operator_policy",
                "weakens": false,
            },
        ])
    );
}

/// An air-gapped Operator policy locks `egress.offline` on: a run that
/// turns it off is refused before it begins, and one that leaves it alone
/// starts (ADRs 0043 and 0083).
///
/// Covers: story:4-82, setting:egress.offline
#[test]
fn an_operator_policy_locking_egress_offline_refuses_a_run_turning_it_off() {
    let env = TestEnv::new();
    write(
        &env,
        "operator-policy.toml",
        "version = \"0.1\"\n\n[lock.egress]\noffline = true\n",
    );

    let (refused, _) = start_and_finish(
        &env,
        Product::Collie,
        json!({ "run_id": "set", "settings": { "egress": { "offline": false } } }),
    );
    let (started, _) = start_and_finish(&env, Product::Collie, json!({ "run_id": "set" }));

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            &refused["error"]["data"]["limit"],
            &started["error"],
        ),
        (
            &json!("setting_locked"),
            &json!({ "key": "egress.offline", "value": true, "layer": "operator_policy" }),
            &Value::Null,
        )
    );
}

/// Story 4-33: `tls.trusted_cas` is the operator's, written in configuration
/// or the Operator policy: a run's own value is refused with
/// `setting_locked` before the Run begins, and the same value in user
/// configuration starts the Run.
/// Covers: story:4-33, error:setting_locked, setting:tls.trusted_cas
#[test]
fn a_runs_trusted_cas_is_locked_and_the_same_value_from_configuration_starts_the_run() {
    let from_the_run = TestEnv::new();
    let from_configuration = TestEnv::new();
    write(
        &from_configuration,
        "configuration.toml",
        "version = \"0.1\"\n\n[settings.tls.trusted_cas]\n\"intranet.example\" = [\"ca.pem\"]\n",
    );

    let (refused, _) = start_and_finish(
        &from_the_run,
        Product::Collie,
        json!({
            "run_id": "set",
            "settings": { "tls": { "trusted_cas": { "intranet.example": ["ca.pem"] } } },
        }),
    );
    let (started, _) = start_and_finish(
        &from_configuration,
        Product::Collie,
        json!({ "run_id": "set" }),
    );

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            &refused["error"]["data"]["limit"]["key"],
        ),
        (&json!("setting_locked"), &json!("tls.trusted_cas"))
    );
    assert_eq!(started["error"], Value::Null, "{started}");
}

/// A refused setting reads in the form the Client's `hello` declares, as
/// does the key closest to one the catalog lacks, which keeps its own
/// spelling; the details name both canonically (ADR 0099).
#[rstest]
#[case::invalid_typescript("typescript", json!({ "first_byte": "soon" }), "`timeout.firstByte` is not valid", "timeout.first_byte", json!(null))]
#[case::invalid_python("python", json!({ "first_byte": "soon" }), "`timeout.first_byte` is not valid", "timeout.first_byte", json!(null))]
#[case::invalid_rust("rust", json!({ "first_byte": "soon" }), "`timeout::FIRST_BYTE` is not valid", "timeout.first_byte", json!(null))]
#[case::unknown_typescript("typescript", json!({ "first_bytes": 1000 }), "`timeout.first_bytes` in run is no setting; did you mean `timeout.firstByte`?", "timeout.first_bytes", json!("timeout.first_byte"))]
#[case::unknown_python("python", json!({ "first_bytes": 1000 }), "`timeout.first_bytes` in run is no setting; did you mean `timeout.first_byte`?", "timeout.first_bytes", json!("timeout.first_byte"))]
#[case::unknown_rust("rust", json!({ "first_bytes": 1000 }), "`timeout.first_bytes` in run is no setting; did you mean `timeout::FIRST_BYTE`?", "timeout.first_bytes", json!("timeout.first_byte"))]
fn a_refused_setting_reads_in_the_declared_form(
    #[case] form: &str,
    #[case] timeout: Value,
    #[case] spelled: &str,
    #[case] key: &str,
    #[case] closest: Value,
) {
    let env = TestEnv::new();
    let mut said = hello("collie", "0.1");
    said["params"]["form"] = json!(form);

    let started = attached_on(
        &engine_with(Product::Collie, None),
        &env,
        |mut client| async move {
            client.send(&[said]).await;
            client.next().await;
            let params = json!({ "run_id": "set", "settings": { "timeout": timeout } });
            call(&mut client, 1, "run.start", params).await
        },
    );

    let message = started["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains(spelled), "{started}");
    assert_eq!(
        (
            &started["error"]["data"]["key"],
            &started["error"]["data"]["closest"],
        ),
        (&json!(key), &closest),
        "{started}"
    );
}

/// Calls `method` on a Daemon of `product` served with `options`, with
/// `settings` as the project file `/work/goodfetch.toml` a Client sends;
/// answers its reply.
fn sent_with_a_project(
    product: Product,
    options: DaemonOptions,
    method: &'static str,
    settings: Value,
) -> Value {
    let (_, answer) = served(
        &engine(product),
        &TestEnv::new(),
        options,
        |dialer| async move {
            let mut client = super::daemon::connect(&dialer).await;
            let said = hello_with(product.as_str(), "0.1", "sdk_typescript", Some(TOKEN));
            client.send(&[said]).await;
            client.next().await;
            let project = json!({ "path": "/work/goodfetch.toml", "settings": settings });
            call(&mut client, 1, method, json!({ "project": project })).await
        },
    );
    answer
}

/// A project file sent to an Exposed Daemon keeps layer 3's rank but has
/// only the run layer's authority there (ADR 0083): a value that widens
/// what ADR 0028 reserves to configuration is refused with
/// `setting_locked`, on `run.start` as on `config.explain`.
///
/// Covers: story:11-75, error:setting_locked
#[rstest]
/// Covers: operation:config.explain, setting:tls.trusted_cas
#[case::trusted_cas_explained(
    Product::Collie,
    "config.explain",
    json!({ "tls": { "trusted_cas": { "intranet.example": ["ca.pem"] } } }),
    "tls.trusted_cas"
)]
/// Covers: operation:run.start, setting:tls.trusted_cas
#[case::trusted_cas_started(
    Product::Collie,
    "run.start",
    json!({ "tls": { "trusted_cas": { "intranet.example": ["ca.pem"] } } }),
    "tls.trusted_cas"
)]
/// Covers: operation:config.explain, setting:tls.accept_invalid_certs
#[case::any_invalid_certificate_explained(
    Product::Collie,
    "config.explain",
    json!({ "tls": { "accept_invalid_certs": { "intranet.example": "any" } } }),
    "tls.accept_invalid_certs"
)]
/// Covers: operation:config.explain, setting:consent.domains
#[case::consent_domains_explained(
    Product::Collie,
    "config.explain",
    json!({ "consent": { "domains": ["news.example"] } }),
    "consent.domains"
)]
/// Covers: operation:config.explain, setting:paid_fetch.domains
#[case::paid_fetch_domains_explained(
    Product::Foxhound,
    "config.explain",
    json!({ "paid_fetch": { "domains": ["shop.example"] } }),
    "paid_fetch.domains"
)]
fn a_project_file_sent_to_an_exposed_daemon_widens_nothing_reserved_to_configuration(
    #[case] product: Product,
    #[case] method: &'static str,
    #[case] settings: Value,
    #[case] key: &str,
) {
    let answer = sent_with_a_project(product, exposed(), method, settings);

    let data = &answer["error"]["data"];
    assert_eq!(
        (&data["code"], &data["key"], &data["layer"]),
        (
            &json!("setting_locked"),
            &json!(key),
            &json!("project_configuration")
        ),
        "{answer}"
    );
}

/// A project file's value is taken at `project_configuration` wherever its
/// authority allows it: a reserved one on this machine's Daemon, and on an
/// Exposed Daemon one the run layer may give, as a domain listed for trust
/// on first use (ADR 0083).
///
/// Covers: story:11-75, operation:config.explain
#[rstest]
/// Covers: setting:tls.trusted_cas
#[case::trusted_cas_on_the_machines_daemon(
    daemon_options(true, &no_handover()),
    json!({ "tls": { "trusted_cas": { "intranet.example": ["ca.pem"] } } }),
    "tls.trusted_cas"
)]
/// Covers: setting:tls.accept_invalid_certs
#[case::trust_on_first_use_on_an_exposed_daemon(
    exposed(),
    json!({ "tls": { "accept_invalid_certs": { "intranet.example": null } } }),
    "tls.accept_invalid_certs"
)]
fn a_project_files_value_is_taken_as_project_configuration_where_its_authority_allows(
    #[case] options: DaemonOptions,
    #[case] settings: Value,
    #[case] key: &str,
) {
    let answer = sent_with_a_project(Product::Collie, options, "config.explain", settings);

    let explained = answer["result"]["settings"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|setting| setting["key"] == json!(key));
    assert_eq!(
        explained.map(|setting| &setting["layer"]),
        Some(&json!("project_configuration")),
        "{answer}"
    );
}
