//! An Operator policy over a crawl's settings (spec #581's story 314; ADRs
//! 0028 and 0086): each registered key of `crawl.*`, `fetch.*`,
//! `politeness.*` and `site_preferences.*`, and `sinks`, can be locked, and each of them
//! that is a count or a duration capped, and no layer above the policy,
//! whether a program's code, the project file, a CLI flag or a Recipe,
//! gets past the lock or the cap. The cases are generated from the keys
//! the catalog registers, so a key added later is checked too.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::catalog::{Product, ValueKind};
use goodfetch_protocol::settings::{RunOrigin, SettingsLayer};
use serde_json::{Value, json};

use super::{catalog_key, policy, recipe, refusal, resolve, resolved, values};
use crate::settings::keys::{self, Key};
use crate::settings::{Layers, SettingsError};

/// The areas whose keys an Operator policy caps or locks for a crawl.
const AREAS: [&str; 5] = ["crawl", "fetch", "politeness", "site_preferences", "sinks"];

/// Every registered key of [`AREAS`].
fn crawl_keys() -> Vec<Key> {
    keys::all()
        .filter(|key| {
            key.setting
                .name
                .split('.')
                .next()
                .is_some_and(|area| AREAS.contains(&area))
        })
        .collect()
}

/// A value of `kind` other than `than`.
fn other(kind: &ValueKind, than: &Value) -> Value {
    let candidates = match kind {
        ValueKind::Boolean => vec![json!(true), json!(false)],
        ValueKind::Count | ValueKind::CountUpTo(_) | ValueKind::CountFromOne => {
            vec![json!(3), json!(4)]
        }
        ValueKind::Duration => vec![json!(1_000), json!(2_000)],
        ValueKind::Text => vec![json!("one"), json!("two")],
        ValueKind::Choice(words) => words.iter().map(|word| json!(word)).collect(),
        ValueKind::List(item) => vec![json!([]), json!([other(item, &Value::Null)])],
        ValueKind::Map(entry) => {
            vec![
                json!({}),
                json!({ "example.test": other(entry, &Value::Null) }),
            ]
        }
        ValueKind::Shape(shape) => (shape.examples)(),
        ValueKind::Optional(inner) => vec![Value::Null, other(inner, &Value::Null)],
        ValueKind::Price => vec![json!("0.05 USD"), json!("0.10 USD")],
        ValueKind::Address => vec![json!("127.0.0.1"), json!("::1")],
        ValueKind::SecretReference => vec![json!("env:ONE"), json!("env:TWO")],
    };
    candidates
        .into_iter()
        .find(|candidate| candidate != than)
        .expect("invariant: every kind a crawl key has holds two values")
}

/// The value an Operator policy locks `key` at: its default, or a value of
/// its kind where it has none.
fn locked_at(key: Key) -> Value {
    let default = (key.setting.default)();
    if default.is_null() {
        other(&key.setting.kind, &Value::Null)
    } else {
        default
    }
}

/// Each layer above the Operator policy, with `value` for `name` there:
/// user and project configuration, a Recipe's declared value, and the run's
/// from a program's code and from a CLI flag.
fn above_the_policy(name: &str, value: &Value) -> Vec<(&'static str, SettingsLayer, Layers)> {
    let set = json!({ name: value });
    let run = |origin| values(SettingsLayer::Run, set.clone()).from_origin(origin);
    vec![
        (
            "user configuration",
            SettingsLayer::UserConfiguration,
            Layers {
                user_configuration: values(SettingsLayer::UserConfiguration, set.clone()),
                ..Layers::default()
            },
        ),
        (
            "the project file",
            SettingsLayer::ProjectConfiguration,
            Layers {
                project_configuration: values(SettingsLayer::ProjectConfiguration, set.clone()),
                ..Layers::default()
            },
        ),
        (
            "a Recipe",
            SettingsLayer::Recipe,
            Layers {
                recipe: Some(recipe(set.clone(), &[])),
                ..Layers::default()
            },
        ),
        (
            "code",
            SettingsLayer::Run,
            Layers {
                run: run(RunOrigin::Settings),
                ..Layers::default()
            },
        ),
        (
            "the CLI",
            SettingsLayer::Run,
            Layers {
                run: run(RunOrigin::Flag),
                ..Layers::default()
            },
        ),
    ]
}

/// Whether `refused` is `setting_locked` from `layer` naming the Operator
/// policy's `limit` on `name`.
fn refused_by_the_policy(
    refused: Result<SettingsError, String>,
    name: &str,
    layer: SettingsLayer,
    limit: &Value,
) -> Result<(), String> {
    match refused? {
        SettingsError::Locked {
            key,
            layer: from,
            limit: found,
            ..
        } if key == name
            && from == layer
            && found.layer == SettingsLayer::OperatorPolicy
            && found.value == *limit =>
        {
            Ok(())
        }
        other => Err(format!("refused otherwise: {other:?}")),
    }
}

/// What resolving `layers` refused, or why it did not.
fn refused(layers: &Layers, name: &str) -> Result<SettingsError, String> {
    match resolve(layers, Product::Foxhound, Product::Foxhound) {
        Ok(resolved) => Err(format!("taken: {}", resolved.value(catalog_key(name)))),
        Err(error) => Ok(error),
    }
}

/// A generated case per registered crawl key: an Operator policy that
/// locks it sets it, says it locked it, and refuses a different value from
/// every layer above it with `setting_locked` naming the lock.
///
/// Covers: story:6-314, error:setting_locked, setting:sinks
#[test]
fn an_operator_policy_locks_every_crawl_key_against_every_higher_layer() {
    let keys = crawl_keys();
    let mut failures = Vec::new();
    for key in &keys {
        let name = key.setting.name;
        let at = locked_at(*key);
        let lock = policy(json!({}), json!({ name: at }), json!({}));
        let alone = resolved(&Layers {
            operator_policy: lock.clone(),
            ..Layers::default()
        });
        let effective = alone
            .effective(catalog_key(name))
            .expect("a crawl key on a Foxhound Run");
        if (&effective.value, effective.layer, effective.locked_by)
            != (
                &at,
                SettingsLayer::OperatorPolicy,
                Some(SettingsLayer::OperatorPolicy),
            )
        {
            failures.push(format!("{name} locked alone: {effective:?}"));
        }
        let changed = other(&key.setting.kind, &at);
        for (from, layer, mut layers) in above_the_policy(name, &changed) {
            // A layer the key's authority keeps it out of, such as a
            // machine's key in the project file, refuses it with no policy.
            if matches!(
                refused(&layers, name),
                Ok(SettingsError::NotApplicable { .. })
            ) {
                continue;
            }
            layers.operator_policy = lock.clone();
            if let Err(why) = refused_by_the_policy(refused(&layers, name), name, layer, &at) {
                failures.push(format!("{name} = {changed} from {from}: {why}"));
            }
        }
    }

    assert!(
        keys.iter()
            .any(|key| key.setting.name == "site_preferences.respect_robots_txt"),
        "the generated cases miss robots.txt"
    );
    assert!(
        keys.iter().any(|key| key.setting.name == "sinks"),
        "the generated cases miss the Sinks"
    );
    assert_eq!(failures, Vec::<String>::new());
}

/// The cap each generated cap case gives.
const CAP: u64 = 7;

/// A generated case per registered crawl key that is a count or a
/// duration: an Operator policy's cap bounds its default, takes a value at
/// the cap from every layer above it, and refuses one past it, or a clear
/// to no bound where the key can be cleared, with `setting_locked` naming
/// the cap.
///
/// Covers: story:6-314, error:setting_locked
#[test]
fn an_operator_policy_caps_every_crawl_count_and_duration_against_every_higher_layer() {
    let capped: Vec<Key> = crawl_keys()
        .into_iter()
        .filter(|key| {
            matches!(
                key.setting.kind,
                ValueKind::Count | ValueKind::CountUpTo(_) | ValueKind::Duration
            )
        })
        .collect();
    let mut failures = Vec::new();
    for key in &capped {
        let name = key.setting.name;
        let cap = policy(json!({}), json!({}), json!({ name: CAP }));
        if let Err(error) = cap.check_caps() {
            failures.push(format!("{name} cap refused: {error:?}"));
            continue;
        }
        let alone = resolved(&Layers {
            operator_policy: cap.clone(),
            ..Layers::default()
        });
        let effective = alone
            .effective(catalog_key(name))
            .expect("a crawl key on a Foxhound Run");
        let within = effective.value.as_u64().is_some_and(|value| value <= CAP);
        if !within || effective.cap != Some(json!(CAP)) {
            failures.push(format!("{name} capped alone: {effective:?}"));
        }
        let mut past = vec![json!(CAP + 1)];
        if key.setting.clearable {
            past.push(Value::Null);
        }
        for value in past {
            for (from, layer, mut layers) in above_the_policy(name, &value) {
                layers.operator_policy = cap.clone();
                if let Err(why) =
                    refused_by_the_policy(refused(&layers, name), name, layer, &json!(CAP))
                {
                    failures.push(format!("{name} = {value} from {from}: {why}"));
                }
            }
        }
        for (from, _, mut layers) in above_the_policy(name, &json!(CAP)) {
            layers.operator_policy = cap.clone();
            if let Ok(error) = refused(&layers, name) {
                failures.push(format!("{name} = {CAP} from {from} refused: {error:?}"));
            }
        }
    }

    assert!(
        capped
            .iter()
            .any(|key| key.setting.name == "crawl.max_requests"),
        "the generated cases miss crawl.max_requests"
    );
    assert_eq!(failures, Vec::<String>::new());
}

/// The run's `crawl.max_requests` past the Operator policy's cap is refused
/// naming the cap, and one within it bounds the Run; with none set, the cap
/// is the Run's bound, set by the policy, which `config explain` names.
///
/// Covers: story:6-314, setting:crawl.max_requests, error:setting_locked
#[test]
fn a_cap_on_crawl_max_requests_bounds_a_run_asking_for_more() {
    let cap = policy(
        json!({}),
        json!({}),
        json!({ "crawl": { "max_requests": 100 } }),
    );
    let asking = |requests: Value| Layers {
        operator_policy: cap.clone(),
        run: values(
            SettingsLayer::Run,
            json!({ "crawl": { "max_requests": requests } }),
        ),
        ..Layers::default()
    };

    let more = refusal(&asking(json!(1_000)));
    let fewer = resolved(&asking(json!(50)));
    let explained = resolved(&Layers {
        operator_policy: cap.clone(),
        ..Layers::default()
    })
    .explain(&["crawl.max_requests".to_owned()], Vec::new());

    let SettingsError::Locked { limit, .. } = &more else {
        panic!("expected setting_locked, got {more:?}");
    };
    let setting = &explained.settings[0];
    assert_eq!(
        (
            (&limit.value, limit.layer),
            fewer.value(catalog_key("crawl.max_requests")),
            (&setting.value, setting.layer, &setting.cap),
        ),
        (
            (&json!(100), SettingsLayer::OperatorPolicy),
            &json!(50),
            (
                &json!(100),
                SettingsLayer::OperatorPolicy,
                &Some(json!(100))
            ),
        )
    );
}
