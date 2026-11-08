//! The `extraction` family's structured pages (task `extraction/10`, spec
//! #580, ticket #787): pages whose metadata and contacts a program reads,
//! each judged as a JSON value, so a JSON-LD block left out, a guessed
//! country's E.164 or an Open Graph tag lost is wrong.
//! Covers: story:2-33

use serde::Deserialize;
use serde_json::{Value, json};

use crate::proving_ground::{ProvingGround, submission};
use crate::reading::{every, origin, start_url, task};

const SEED: u64 = 787;
const TASK: &str = "extraction/10";

#[derive(Deserialize)]
struct Pages {
    pages: Vec<Page>,
}

#[derive(Deserialize)]
#[expect(dead_code, reason = "the served HTML is the server's")]
struct Page {
    path: String,
    html: String,
    metadata: String,
    contacts: String,
}

/// One Record per committed page, its formats as JSON values with the
/// task's origin in place, then changed by `change`.
fn records(origin: &str, change: impl Fn(&str, &mut Value)) -> Vec<Value> {
    toml::from_str::<Pages>(include_str!("../../pages/structured.toml"))
        .expect("the committed structured pages parse")
        .pages
        .iter()
        .map(|page| {
            let id = format!("/structured/{}", page.path);
            let value = |text: &str| -> Value {
                serde_json::from_str(&text.replace("{origin}", origin))
                    .expect("each committed format is JSON")
            };
            let mut record = json!({
                "id": id,
                "metadata": value(&page.metadata),
                "contacts": value(&page.contacts),
            });
            change(&id, &mut record);
            record
        })
        .collect()
}

#[test]
fn the_structured_pages_are_listed_and_served() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, TASK);
    let origin = origin(start_url(&task)).to_owned();

    let index = proving_ground.get(start_url(&task)).body;
    let listed = every(r#"href="(/structured/[^"]+)""#, &index);
    let served: Vec<u16> = listed
        .iter()
        .map(|path| proving_ground.get(&format!("{origin}{path}")).status)
        .collect();

    assert_eq!(
        (listed.as_slice(), served),
        (
            [
                "/structured/product",
                "/structured/no-region",
                "/structured/region"
            ]
            .map(String::from)
            .as_slice(),
            vec![200, 200, 200]
        )
    );
}

#[test]
fn every_pages_metadata_and_contacts_as_json_values_are_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let origin = origin(start_url(&task(&proving_ground, TASK))).to_owned();

    let answer = proving_ground.judge(&submission(TASK, "success", &records(&origin, |_, _| {})));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// A JSON-LD block left out, an E.164 guessed for a number with no
/// country code on a page whose language tag has no region, and an Open
/// Graph tag left out are each wrong.
#[test]
fn a_json_ld_block_left_out_a_guessed_country_or_a_tag_left_out_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let origin = origin(start_url(&task(&proving_ground, TASK))).to_owned();

    let answer = proving_ground.judge(&submission(
        TASK,
        "success",
        &records(&origin, |id, record| match id {
            "/structured/product" => {
                record["metadata"]["json_ld"]
                    .as_array_mut()
                    .unwrap()
                    .remove(1);
            }
            "/structured/no-region" => {
                record["contacts"]["phones"][0]["e164"] = json!("+442070313000");
            }
            "/structured/region" => {
                record["metadata"]["open_graph"] = json!([]);
            }
            _ => {}
        }),
    ));

    let wrong: Vec<(&Value, &Value)> = answer["wrong"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|wrong| (&wrong["id"], &wrong["field"]))
        .collect();
    assert_eq!(
        (&answer["outcome"], wrong),
        (
            &json!("false_success"),
            vec![
                (&json!("/structured/product"), &json!("metadata")),
                (&json!("/structured/no-region"), &json!("contacts")),
                (&json!("/structured/region"), &json!("metadata")),
            ]
        ),
        "{answer}"
    );
}
