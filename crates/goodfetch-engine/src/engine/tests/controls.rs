//! Switchable controls through the engine's public interface over the
//! in-memory network (spec #213's Seam 3; ADRs 0016, 0028 and 0102): a Run
//! that starts with a control switched off is reported once, however often
//! its Run ID is started again: one chained `security_switch_turned_off`
//! entry in the Audit log, one warning through the host's logging, and the
//! control in its Run report's `weakened`. With `security.audit_chain` off,
//! its other entries, here the use of its key-encryption key, are written
//! unchained and the log still verifies. A control another composition
//! registers, as Foxhound's may, is reported the same way, and a setting no
//! composition registers only weakens.
#![expect(
    clippy::disallowed_types,
    reason = "the test's `tracing` layer collects warnings from the engine's threads"
)]

use std::ffi::OsString;
use std::sync::{Arc, Mutex};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::RUN_STATE_DURABILITY;
use goodfetch_test_support::{Canaries, SecretKind, TestEnv};
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use super::run::call;
use super::{attached_with, dependencies, engine_with, hello};
use crate::audit::AuditLog;
use crate::composition::Contribution;
use crate::engine::{Dependencies, Engine};
use crate::home::Home;

/// The variable the Run's key-encryption key is read from.
const KEY_VARIABLE: &str = "GF_TEST_RUN_KEY";

/// The `setting` field of each warning the engine logged.
#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Warnings {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        struct Setting(String);
        impl tracing::field::Visit for Setting {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "setting" {
                    self.0 = format!("{value:?}");
                }
            }
        }
        if *event.metadata().level() == tracing::Level::WARN {
            let mut setting = Setting(String::new());
            event.record(&mut setting);
            self.0.lock().unwrap().push(setting.0);
        }
    }
}

/// `env`'s dependencies, whose environment holds the Run's key, which the
/// user configuration names by reference.
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the Goodfetch home's user configuration itself"
)]
fn keyed(env: &TestEnv) -> Dependencies {
    let folder = env.home().join("1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("configuration.toml"),
        format!(
            "version = \"0.1\"\n\n[settings]\nrun_state.key_reference = \"env:{KEY_VARIABLE}\"\n"
        ),
    )
    .unwrap();
    let mut canaries = Canaries::new(env.seed());
    let key = canaries.plant(SecretKind::RunDataKey).value().to_owned();
    Dependencies {
        environment: [(OsString::from(KEY_VARIABLE), OsString::from(key))].into(),
        ..dependencies(env)
    }
}

/// What a Run of `engine` that starts twice under the Run ID `switched`
/// with `settings`, its key named by reference, then finishes, left: its
/// report, the warnings logged, and each Audit log entry as its kind, its
/// Run, whether it is chained and the setting it names.
fn started_twice(
    engine: &Engine,
    env: &TestEnv,
    settings: &Value,
) -> (Value, Vec<String>, Vec<Value>) {
    let params = json!({ "run_id": "switched", "settings": settings });
    let warnings = Warnings::default();
    let subscriber = tracing_subscriber::registry().with(warnings.clone());
    let finished = tracing::subscriber::with_default(subscriber, || {
        attached_with(
            engine,
            || keyed(env),
            |mut client| async move {
                client.send(&[hello("collie", "0.1")]).await;
                client.next().await;
                call(&mut client, 1, "run.start", params.clone()).await;
                call(&mut client, 2, "run.start", params).await;
                call(
                    &mut client,
                    3,
                    "run.finish",
                    json!({ "run_id": "switched" }),
                )
                .await
            },
        )
    });
    let log = AuditLog::of(&Home::new(env.home().to_owned()), None);
    let entries = log
        .export()
        .unwrap()
        .into_iter()
        .map(|exported| {
            json!([
                exported.entry.kind,
                exported.entry.run_id,
                exported.entry.chained,
                exported.entry.details["setting"],
            ])
        })
        .collect();
    assert!(log.verify().unwrap().intact, "the Audit log verifies");
    let warnings = warnings.0.lock().unwrap().clone();
    (finished["result"].clone(), warnings, entries)
}

/// Covers: story:4-74, setting:security.audit_chain, setting:run_state.key_reference
#[test]
fn a_run_with_the_audit_chain_switched_off_is_reported_once_and_its_entries_written_unchained() {
    let env = TestEnv::new();

    let (report, warnings, entries) = started_twice(
        &engine_with(Product::Collie, None),
        &env,
        &json!({ "security": { "audit_chain": false } }),
    );

    assert_eq!(
        (&report["weakened"], warnings, entries),
        (
            &json!([{
                "key": "security.audit_chain",
                "value": false,
                "layer": "run",
                "origin": "settings",
            }]),
            vec!["security.audit_chain".to_owned()],
            vec![
                json!(["secret_reference_used", "switched", false, null]),
                json!([
                    "security_switch_turned_off",
                    "switched",
                    true,
                    "security.audit_chain"
                ]),
                json!(["secret_reference_used", "switched", false, null]),
            ]
        )
    );
}

/// The positive control: with every control on, nothing is reported, the
/// report has no `weakened`, and the Run's entries are chained.
/// Covers: story:4-74, setting:security.audit_chain
#[test]
fn a_run_with_every_control_on_reports_no_switch_and_chains_its_entries() {
    let env = TestEnv::new();

    let (report, warnings, entries) =
        started_twice(&engine_with(Product::Collie, None), &env, &json!({}));

    assert_eq!(
        (&report["weakened"], warnings, entries),
        (
            &Value::Null,
            Vec::<String>::new(),
            vec![
                json!(["secret_reference_used", "switched", true, null]),
                json!(["secret_reference_used", "switched", true, null]),
            ]
        )
    );
}

/// A further control a composition registers, as Foxhound's may, is
/// reported as the engine's own are; on an engine whose composition does
/// not register it, the same value only weakens the Run.
/// Covers: story:4-74, setting:run_state.durability
#[test]
fn a_control_another_composition_registers_is_reported_as_the_engines_own() {
    let (registered, unregistered) = (TestEnv::new(), TestEnv::new());
    let fast = json!({ "run_state": { "durability": "fast" } });
    let registering = Contribution::new("test-controls").with_control(&RUN_STATE_DURABILITY);

    let reported = started_twice(
        &engine_with(Product::Collie, Some(registering)),
        &registered,
        &fast,
    );
    let only_weakened = started_twice(&engine_with(Product::Collie, None), &unregistered, &fast);

    let weakened = json!([{
        "key": "run_state.durability",
        "value": "fast",
        "layer": "run",
        "origin": "settings",
    }]);
    assert_eq!(
        (
            (&reported.0["weakened"], reported.1, reported.2),
            (
                &only_weakened.0["weakened"],
                only_weakened.1,
                only_weakened.2
            ),
        ),
        (
            (
                &weakened,
                vec!["run_state.durability".to_owned()],
                vec![
                    json!(["secret_reference_used", "switched", true, null]),
                    json!([
                        "security_switch_turned_off",
                        "switched",
                        true,
                        "run_state.durability"
                    ]),
                    json!(["secret_reference_used", "switched", true, null]),
                ]
            ),
            (
                &weakened,
                Vec::<String>::new(),
                vec![
                    json!(["secret_reference_used", "switched", true, null]),
                    json!(["secret_reference_used", "switched", true, null]),
                ]
            ),
        )
    );
}
