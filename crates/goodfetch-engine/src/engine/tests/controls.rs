//! Switchable controls through the engine's public interface over the
//! in-memory network (spec #213's Seam 3; ADRs 0016, 0028 and 0102): a Run
//! that starts with a control switched off is reported once, however often
//! its Run ID is started again: one chained `security_switch_turned_off`
//! entry in the Audit log, one warning through the host's logging, and the
//! control in its Run report's `weakened`. With `security.audit_chain` off,
//! its other entries, here the use of its key-encryption key, are written
//! unchained and the log still verifies. A control another composition
//! registers, as Foxhound's may, is reported the same way, and a setting no
//! composition registers only weakens. A key a composition registers as a
//! weakening of its own, as Foxhound's Politeness values are, is recorded
//! as the kind it names, with its value, its built-in default and the
//! layer that set it, once per Run, and warned of once; a value that does
//! not weaken records nothing.
#![expect(
    clippy::disallowed_types,
    reason = "the test's `tracing` layer collects warnings from the engine's threads"
)]

use std::ffi::OsString;
use std::sync::{Arc, Mutex};

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, SharedType, SharedTypeValue,
    Stability, schema_of,
};
use goodfetch_protocol::politeness::POLITENESS_SHARING;
use goodfetch_protocol::run::{RUN_STATE_DURABILITY, RunId};
use goodfetch_test_support::{Canaries, SecretKind, TestEnv};
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use super::run::call;
use super::{attached_with, dependencies, engine_with, hello};
use crate::audit::AuditLog;
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::engine::{Dependencies, Engine};
use crate::home::Home;
use crate::known_certificates::ExceptionKind;
use crate::known_certificates::KeyHash;

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

/// A start with no Run ID of its own, whose engine generates one, writes
/// its first entry, its key-encryption key's use before the Run exists,
/// unchained and naming that Run, as the rest of its entries are.
/// Covers: story:4-74, setting:security.audit_chain, setting:run_state.key_reference
#[test]
fn a_generated_runs_first_entry_is_unchained_with_the_audit_chain_off() {
    let env = TestEnv::new();
    let params = json!({ "settings": { "security": { "audit_chain": false } } });

    let started = attached_with(
        &engine_with(Product::Collie, None),
        || keyed(&env),
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.start", params).await
        },
    );

    let run_id = started["result"]["run_id"].clone();
    let entries: Vec<Value> = AuditLog::of(&Home::new(env.home().to_owned()), None)
        .export()
        .unwrap()
        .into_iter()
        .map(|exported| {
            json!([
                exported.entry.kind,
                exported.entry.run_id,
                exported.entry.chained
            ])
        })
        .collect();
    assert_eq!(
        entries,
        vec![
            json!(["secret_reference_used", run_id, false]),
            json!(["security_switch_turned_off", run_id, true]),
        ],
        "{started}"
    );
}

/// A resume whose settings switch a control off is reported as a start's
/// are (ADR 0016): one chained `security_switch_turned_off` entry, one
/// warning and the control in the Run report's `weakened`, for each control
/// the Run's report did not list yet; a control its start switched off, and
/// a second resume with the same settings, add nothing.
/// Covers: story:4-74
#[test]
fn a_resume_that_switches_a_control_off_is_reported_once_as_a_start_is() {
    let env = TestEnv::new();
    let started =
        json!({ "run_id": "resumed", "settings": { "security": { "redaction": false } } });
    let resumed = json!({
        "run_id": "resumed",
        "settings": { "security": { "redaction": false, "resolved_ip_check": false } },
    });
    let paused = json!({ "run_id": "resumed" });
    let warnings = Warnings::default();
    let subscriber = tracing_subscriber::registry().with(warnings.clone());

    let finished = tracing::subscriber::with_default(subscriber, || {
        attached_with(
            &engine_with(Product::Collie, None),
            || dependencies(&env),
            |mut client| async move {
                client.send(&[hello("collie", "0.1")]).await;
                client.next().await;
                call(&mut client, 1, "run.start", started).await;
                call(&mut client, 2, "run.pause", paused.clone()).await;
                call(&mut client, 3, "run.resume", resumed.clone()).await;
                call(&mut client, 4, "run.pause", paused.clone()).await;
                call(&mut client, 5, "run.resume", resumed).await;
                call(&mut client, 6, "run.finish", paused).await
            },
        )
    });

    let switched: Vec<Value> = recorded(&env);
    assert_eq!(
        (
            &finished["result"]["weakened"],
            warnings.0.lock().unwrap().clone(),
            switched,
        ),
        (
            &json!([
                { "key": "security.redaction", "value": false, "layer": "run", "origin": "settings" },
                { "key": "security.resolved_ip_check", "value": false, "layer": "run", "origin": "settings" },
            ]),
            vec![
                "security.redaction".to_owned(),
                "security.resolved_ip_check".to_owned()
            ],
            vec![
                json!([
                    "security_switch_turned_off",
                    "resumed",
                    "run",
                    { "setting": "security.redaction", "value": false },
                ]),
                json!([
                    "security_switch_turned_off",
                    "resumed",
                    "run",
                    { "setting": "security.resolved_ip_check", "value": false },
                ]),
            ],
        ),
        "{finished}"
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

/// Each Audit log entry of `env`'s home that is not a key reference's use,
/// as its kind, its Run, the layer that set it and its details.
fn recorded(env: &TestEnv) -> Vec<Value> {
    AuditLog::of(&Home::new(env.home().to_owned()), None)
        .export()
        .unwrap()
        .into_iter()
        .filter(|exported| exported.entry.kind != "secret_reference_used")
        .map(|exported| {
            json!([
                exported.entry.kind,
                exported.entry.run_id,
                exported.entry.layer,
                exported.entry.details,
            ])
        })
        .collect()
}

/// The Audit log kind the test's composition records its weakening as.
const PRESSED_HARDER: SharedTypeValue = SharedTypeValue {
    shared_type: SharedType::AuditKind,
    name: "test_pressed_harder",
    description: "A run set a value that presses a site harder than its built-in default.",
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

/// A key a composition registers as a weakening recorded as a kind of its
/// own, as Foxhound's composition registers its Politeness values (ADR
/// 0086), is recorded once per Run as that kind, with its value, its
/// built-in default and the layer that set it, warned of once and listed
/// in `weakened`, however often the Run starts; its default records
/// nothing (positive control).
/// Covers: story:4-117, setting:politeness.sharing
#[test]
fn a_registered_weakening_is_recorded_once_as_its_kind_with_its_default_and_layer() {
    let (pressing, gentle) = (TestEnv::new(), TestEnv::new());
    let registering = || {
        Contribution::new("test-politeness").with_recorded_weakening(
            &POLITENESS_SHARING,
            &PRESSED_HARDER,
            POLITENESS_SHARING.default,
        )
    };

    let pressed = started_twice(
        &engine_with(Product::Collie, Some(registering())),
        &pressing,
        &json!({ "politeness": { "sharing": "run" } }),
    );
    let gentler = started_twice(
        &engine_with(Product::Collie, Some(registering())),
        &gentle,
        &json!({ "politeness": { "sharing": "shared" } }),
    );

    assert_eq!(
        (
            (&pressed.0["weakened"], pressed.1, recorded(&pressing)),
            (&gentler.0["weakened"], gentler.1, recorded(&gentle)),
        ),
        (
            (
                &json!([{
                    "key": "politeness.sharing",
                    "value": "run",
                    "layer": "run",
                    "origin": "settings",
                }]),
                vec!["politeness.sharing".to_owned()],
                vec![json!([
                    "test_pressed_harder",
                    "switched",
                    "run",
                    { "setting": "politeness.sharing", "value": "run", "default": "shared" },
                ])],
            ),
            (&Value::Null, Vec::<String>::new(), Vec::<Value>::new()),
        )
    );
}

/// Marks the hosts `params` lists reached by the live Run `run_id`
/// through an invalid-certificate exception, as its Sessions' TLS
/// exceptions do once they admit a host.
const REACH: Operation = Operation {
    name: "test.reach",
    description: "Marks hosts reached through a certificate exception.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

async fn reach(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
    let run = context.runs().live(run_id).await.unwrap();
    for (host, kind) in [
        ("pinned.test", ExceptionKind::TrustOnFirstUse),
        ("any.test", ExceptionKind::Any),
        ("pinned.test", ExceptionKind::TrustOnFirstUse),
    ] {
        run.secrets()
            .exception_hosts()
            .add(host, KeyHash::of_public_key(host.as_bytes()), kind)
            .await;
    }
    Ok(json!({}))
}

/// A Run that reached hosts through certificate exceptions records them in
/// the Audit log once, as it ends, by host and kind with each key (ADR
/// 0042); the Runs above, which reached none, record no such entry.
/// Covers: story:4-37, setting:tls.accept_invalid_certs
#[test]
fn a_run_records_the_certificate_exceptions_it_used_once_as_it_ends() {
    let env = TestEnv::new();
    let engine = engine_with(
        Product::Collie,
        Some(Contribution::new("test-reach").with_handler(Handler::new(&REACH, reach))),
    );

    attached_with(
        &engine,
        || dependencies(&env),
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.start", json!({ "run_id": "reaching" })).await;
            call(
                &mut client,
                2,
                "test.reach",
                json!({ "run_id": "reaching" }),
            )
            .await;
            call(
                &mut client,
                3,
                "run.finish",
                json!({ "run_id": "reaching" }),
            )
            .await
        },
    );

    let key = |host: &str| KeyHash::of_public_key(host.as_bytes()).to_string();
    assert_eq!(
        recorded(&env),
        vec![json!([
            "certificate_exceptions_used",
            "reaching",
            null,
            { "hosts": [
                { "host": "any.test", "kind": "any", "spki_sha256": key("any.test") },
                { "host": "pinned.test", "kind": "trust_on_first_use", "spki_sha256": key("pinned.test") },
            ] },
        ])]
    );
}
