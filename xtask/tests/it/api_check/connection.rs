//! The Collie client packages compared with their own previous release
//! (ADR 0095, issue 148): beside Collie's projection, which `api-check`
//! compares, and their hand-written code, which `sdk-api` and
//! `semver-checks` compare, they carry the bundle's `connection` section,
//! the messages every Connection speaks. Each message is compared by the
//! side its data flows, and a message both peers send may neither narrow
//! nor widen.
//! Covers: story:1-29

use serde_json::{Value, json};

use super::{BUNDLE, edit_json, expected, released_with};
use crate::workspace::Workspace;

/// An object schema with one required string property, `name`.
fn object(name: &str) -> Value {
    json!({
        "properties": {name: {"type": "string"}},
        "required": [name],
        "type": "object"
    })
}

/// The fixture with a `connection` section of one message per side,
/// released.
fn released() -> Workspace {
    released_with(
        |workspace| {
            edit_json(workspace, BUNDLE, |bundle| {
                bundle["connection"] = json!({
                    "hello": {
                        "params": {"$ref": "#/$defs/HelloParams"},
                        "result": {"$ref": "#/$defs/HelloResult"}
                    },
                    "events": {"params": {"$ref": "#/$defs/EventBatch"}},
                    "seeds": {"params": {"$ref": "#/$defs/SeedBatch"}},
                    "meta": {"$ref": "#/$defs/RequestMeta"}
                });
                let definitions = &mut bundle["$defs"];
                definitions["HelloResult"] = object("engine_version");
                definitions["EventBatch"] = object("run_id");
                definitions["SeedBatch"] = object("url");
                definitions["RequestMeta"] = object("traceparent");
            });
        },
        "",
    )
}

#[rstest::rstest]
#[case::unchanged("hello", |_: &mut Value| {}, false)]
#[case::a_message_removed("events", |bundle: &mut Value| {
    bundle["connection"].as_object_mut().unwrap().remove("events");
}, true)]
#[case::a_message_added("chunk", |bundle: &mut Value| {
    bundle["connection"]["chunk"] = json!({"params": {"type": "object"}});
}, false)]
#[case::a_field_the_engine_answers_added("hello", |bundle: &mut Value| {
    bundle["$defs"]["HelloResult"]["properties"]["features"] = json!({"type": "array"});
}, false)]
#[case::a_field_the_engine_sends_removed("events", |bundle: &mut Value| {
    bundle["$defs"]["EventBatch"] = json!({"type": "object"});
}, true)]
#[case::a_field_the_client_sends_now_required("seeds", |bundle: &mut Value| {
    let batch = &mut bundle["$defs"]["SeedBatch"];
    batch["properties"]["depth"] = json!({"type": "integer"});
    batch["required"] = json!(["url", "depth"]);
}, true)]
#[case::a_field_the_client_sends_made_optional("seeds", |bundle: &mut Value| {
    bundle["$defs"]["SeedBatch"]["required"] = json!([]);
}, false)]
#[case::a_field_the_engine_reads_widened("hello", |bundle: &mut Value| {
    bundle["$defs"]["HelloParams"]["properties"]["protocol_version"]["type"] = json!(["string", "null"]);
}, false)]
#[case::a_field_both_peers_send_widened("meta", |bundle: &mut Value| {
    bundle["$defs"]["RequestMeta"]["properties"]["traceparent"]["type"] = json!(["string", "null"]);
}, true)]
#[case::a_field_both_peers_send_made_optional("meta", |bundle: &mut Value| {
    bundle["$defs"]["RequestMeta"]["required"] = json!([]);
}, true)]
#[case::a_field_both_peers_send_added_as_optional("meta", |bundle: &mut Value| {
    bundle["$defs"]["RequestMeta"]["properties"]["raw"] = json!({"type": "boolean"});
}, false)]
fn a_connection_message_change_by_the_side_its_data_flows(
    #[case] message: &str,
    #[case] change: fn(&mut Value),
    #[case] breaks: bool,
) {
    let workspace = released();
    edit_json(&workspace, BUNDLE, change);

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(
        outcome.findings(),
        expected(breaks, &format!("collie-client connection {message}")),
        "{}",
        outcome.stdout
    );
}

#[test]
fn an_announced_break_of_a_connection_message_passes() {
    let workspace = released();
    workspace.write(
        super::API,
        format!(
            "{}[[announced]]\nentry = \"collie-client connection events\"\nreason = \"Batches carry a Run's events from 2.0.\"\n",
            workspace.read_text(super::API)
        )
        .as_bytes(),
    );
    edit_json(&workspace, BUNDLE, |bundle| {
        bundle["connection"]
            .as_object_mut()
            .unwrap()
            .remove("events");
    });

    let outcome = workspace.xtask(&["api-check"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stdout);
}
