//! The `extraction` family's reference pages (task `extraction/9`, spec
//! #580, ticket #786; pitfall U08): pages whose links, images, media and
//! tables a program reads, each judged as a JSON value, so a URL resolved
//! against the wrong base, a URL that does not parse given in place of
//! `null` or a row header dropped is wrong.
//! Covers: story:2-33

use serde::Deserialize;
use serde_json::{Value, json};

use crate::proving_ground::ProvingGround;
use crate::reading::{every, judge, origin, start_url, task};

const SEED: u64 = 786;
const TASK: &str = "extraction/9";

#[derive(Deserialize)]
struct Pages {
    pages: Vec<Page>,
}

#[derive(Deserialize)]
#[expect(dead_code, reason = "the served HTML and the move are the server's")]
struct Page {
    path: String,
    moves_to: Option<String>,
    html: String,
    links: String,
    images: String,
    media: String,
    tables: String,
}

/// One Record per committed page, its formats as JSON values with the
/// task's origin in place, then changed by `change`.
fn records(origin: &str, change: impl Fn(&str, &mut Value)) -> Vec<Value> {
    toml::from_str::<Pages>(include_str!("../../pages/references.toml"))
        .expect("the committed reference pages parse")
        .pages
        .iter()
        .map(|page| {
            let id = format!("/references/{}", page.path);
            let value = |text: &str| -> Value {
                serde_json::from_str(&text.replace("{origin}", origin))
                    .expect("each committed format is JSON")
            };
            let mut record = json!({
                "id": id,
                "links": value(&page.links),
                "images": value(&page.images),
                "media": value(&page.media),
                "tables": value(&page.tables),
            });
            change(&id, &mut record);
            record
        })
        .collect()
}

#[test]
fn the_reference_pages_are_listed_and_served_and_the_moved_one_answers_a_redirect() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, TASK);
    let origin = origin(start_url(&task)).to_owned();

    let index = proving_ground.get(start_url(&task)).body;
    let listed = every(r#"href="(/references/[^"]+)""#, &index);
    let moved = proving_ground.get(&format!("{origin}/references/moved"));
    let landed = proving_ground.get(&format!("{origin}/references/landed/shelf/"));
    let served: Vec<u16> = ["/references/base", "/references/tables"]
        .iter()
        .map(|path| proving_ground.get(&format!("{origin}{path}")).status)
        .collect();

    assert_eq!(
        listed,
        [
            "/references/base",
            "/references/moved",
            "/references/tables"
        ]
    );
    assert_eq!(
        (moved.status, moved.header("location")),
        (302, Some("/references/landed/shelf/"))
    );
    assert_eq!(
        (landed.status, landed.body.contains("The shelf"), served),
        (200, true, vec![200, 200])
    );
}

#[test]
fn every_pages_formats_as_json_values_are_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let origin = origin(start_url(&task(&proving_ground, TASK))).to_owned();

    let answer = judge(
        &proving_ground,
        TASK,
        "success",
        &records(&origin, |_, _| {}),
    );

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// The moved page's links resolved against the URL before the redirect,
/// a URL that does not parse given as written in place of `null`, and a
/// row header dropped from its row are each wrong.
#[test]
fn a_wrong_base_a_broken_url_given_or_a_row_header_dropped_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let origin = origin(start_url(&task(&proving_ground, TASK))).to_owned();

    let answer = judge(
        &proving_ground,
        TASK,
        "success",
        &records(&origin, |id, record| match id {
            "/references/moved" => {
                record["links"][0]["url"] = json!(format!("{origin}/references/book/1"));
            }
            "/references/base" => {
                record["links"][5]["url"] = json!("http://[::1/");
            }
            "/references/tables" => {
                let row = record["tables"][0]["body_rows"][0].as_array_mut().unwrap();
                row.remove(0);
            }
            _ => {}
        }),
    );

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
                (&json!("/references/base"), &json!("links")),
                (&json!("/references/moved"), &json!("links")),
                (&json!("/references/tables"), &json!("tables")),
            ]
        ),
        "{answer}"
    );
}
