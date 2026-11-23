//! The `head/` set's Checkpoint (ADR 0066) resumed on Foxhound's
//! executable against the Proving ground (story 3-34; ADR 0106): its Run's
//! program runs its body again and declares each Effect it performs; the
//! Effect journal answers the order the Run placed before its Checkpoint
//! instead of letting it be placed again, and judges the Effect a body
//! left sent, which it cannot check, uncertain. The oracle of the Proving
//! ground's `catalog/3` then finds the order placed once.
#![expect(
    clippy::disallowed_types,
    reason = "the program places its order on the Proving ground from outside the engine (ADR 0060)"
)]

use std::io::{Read as _, Write as _};
use std::path::Path;

use goodfetch_test_support::{ProvingGround, TestEnv, ToolStatus};
use serde_json::{Value, json};

use crate::attached;
use crate::fetch_tiers::settings;

/// The Proving ground's seed.
const SEED: u64 = 34;

/// The task whose order the Run placed: one order of the product its order
/// form names.
pub(crate) const ORDER_TASK: &str = "catalog/3";

/// The Run of the `head/` set's Checkpoint.
const RUN_ID: &str = "paused";

/// Places the order the form at `order_form`, the start URL of
/// [`ORDER_TASK`], names, as its form posts it: `product=P-NNNN` to
/// `/orders` on the form's host. Answers the response's status.
pub(crate) fn place_order(proving_ground: &ProvingGround, order_form: &str) -> u16 {
    let authority_and_path = order_form
        .strip_prefix("http://")
        .expect("the order form is served over HTTP");
    let (authority, path) = authority_and_path.split_at(
        authority_and_path
            .find('/')
            .expect("the order form has a path"),
    );
    let product = path
        .split('/')
        .find(|segment| segment.starts_with("P-"))
        .expect("the order form names its product");
    let body = format!("product={product}");
    let mut stream = std::net::TcpStream::connect(proving_ground.address()).unwrap();
    write!(
        stream,
        "POST /orders HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
        .split(' ')
        .nth(1)
        .and_then(|status| status.parse().ok())
        .expect("the Proving ground answers with a status line")
}

/// The orders the Proving ground carried out for [`ORDER_TASK`].
pub(crate) fn orders(proving_ground: &ProvingGround) -> Vec<Value> {
    proving_ground.observations().unwrap()["effects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|effect| effect["task"] == ORDER_TASK)
        .cloned()
        .collect()
}

/// The `head/` set's Checkpoint, resumed on Foxhound's executable with the
/// Proving ground's hosts mapped, holds an order its program declared and
/// closed and an Effect a body left sent. Its program declares both again:
/// the order is answered from the journal, so its body never runs and the
/// Proving ground holds the one order placed before the Checkpoint, which
/// the oracle verifies; the other is `effect_uncertain`, and the Run
/// report lists it and needs review.
/// Covers: story:3-34, operation:run.resume, operation:effect.open, error:effect_uncertain
#[test]
fn the_head_sets_checkpoint_resumes_against_the_proving_ground_and_never_repeats_an_effect() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let order_form = proving_ground.start_url(ORDER_TASK).unwrap();
    let head = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../compat/run-state/head");
    let checkpoint = env.folder().join("checkpoint.sqlite");
    std::fs::copy(head.join("checkpoint.sqlite"), &checkpoint).unwrap();
    // The order the Run placed before its Checkpoint, which its journal
    // holds as performed.
    let placed = place_order(&proving_ground, &order_form);

    let mut engine = attached(&env);
    let resumed = engine.ask(
        2,
        "run.resume",
        &json!({
            "run_id": RUN_ID,
            "checkpoint": checkpoint.display().to_string(),
            "settings": settings(&proving_ground, &json!({})),
        }),
    );
    let order = engine.ask(
        3,
        "effect.open",
        &json!({ "run_id": RUN_ID, "key": "order" }),
    );
    if order["result"]["answer"] == "perform" {
        place_order(&proving_ground, &order_form);
    }
    let label = engine.ask(
        4,
        "effect.open",
        &json!({ "run_id": RUN_ID, "key": "label" }),
    );
    let finished = engine.ask(5, "run.finish", &json!({ "run_id": RUN_ID }));
    assert!(engine.finished());
    let report = &finished["result"];
    let verdict = proving_ground
        .judge(ORDER_TASK, ToolStatus::NeedsReview)
        .unwrap();

    assert_eq!(
        (
            placed,
            &resumed["result"]["started_as"],
            &order["result"]["answer"],
            &order["result"]["output"],
            &label["error"]["data"]["code"],
            &label["error"]["data"]["reason"],
            report["uncertain_effects"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|effect| (effect["key"].clone(), effect["reason"].clone()))
                .collect::<Vec<_>>(),
            &report["verdict"],
            &verdict["outcome"],
            &verdict["extra_effects"],
            orders(&proving_ground).len(),
        ),
        (
            200,
            &json!("resumed"),
            &json!("journal"),
            &json!({ "order": "placed" }),
            &json!("effect_uncertain"),
            &json!("interrupted"),
            vec![(json!("label"), json!("interrupted"))],
            &json!("needs_review"),
            &json!("verified"),
            &json!([]),
            1,
        ),
        "{resumed} {order} {label} {finished} {verdict}"
    );
}
