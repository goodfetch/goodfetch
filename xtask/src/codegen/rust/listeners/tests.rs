use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Form;
use goodfetch_protocol::entries::MODULES;
use goodfetch_protocol::naming;
use serde_json::Value;

use super::marker_name;
use crate::codegen::handles::{self, Listened};
use crate::codegen::layer::Layer;
use crate::codegen::{emit, projection};

/// How the generated SDK of `form` names `listened`: the TypeScript and
/// Python listener's value, or the Rust marker type in the `events` module.
fn generated(listened: &Listened, form: Form) -> String {
    match form {
        Form::Rust => format!(
            "events::{}",
            marker_name(listened).expect("every listened event has a Rust marker")
        ),
        _ => listened.spelled(form),
    }
}

/// Each event a handle's listener reads is named in each SDK as the
/// engine spells it in an error's message (ADR 0099), so the name a
/// program writes and the one the engine tells it of never differ.
#[test]
fn each_listened_event_is_named_as_the_engine_spells_it() {
    let emitted = emit::emit();
    let collie = projection::project(&emitted.catalog, &emitted.bundle, Product::Collie).unwrap();
    let layer = Layer::new(&emitted.bundle, &collie, &Value::Null, Product::Collie, "").unwrap();

    let mut spelled = Vec::new();
    for handle in handles::handles(&layer).unwrap() {
        for listener in handle.listener.iter().chain(&handle.published_listener) {
            for listened in &listener.events {
                let name = format!("{}.{}", handle.namespace, listened.wire);
                let event = MODULES
                    .iter()
                    .flat_map(|entries| entries.events)
                    .find(|event| event.name == name)
                    .unwrap_or_else(|| panic!("the catalog lacks `{name}`"));
                let (engine_name, payload) = naming::listened(event);
                for form in [Form::Typescript, Form::Python, Form::Rust] {
                    spelled.push((
                        format!("{name} in {form:?}"),
                        generated(listened, form),
                        naming::event(form, engine_name, payload.as_deref().unwrap_or("")),
                    ));
                }
            }
        }
    }

    assert!(!spelled.is_empty(), "no handle listens to an event");
    let differing: Vec<_> = spelled
        .into_iter()
        .filter(|(_, generated, engine)| generated != engine)
        .collect();
    assert_eq!(differing, Vec::new());
}
