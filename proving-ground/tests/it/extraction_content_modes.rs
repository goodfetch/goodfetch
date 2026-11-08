//! The `extraction` family's content-mode pages (task `extraction/8`, spec
//! #580): pages read in every form `page.formats` gives, each judged in its
//! format, Markdown by its structure, HTML by the text of its blocks and
//! text white space aside, so a serializer's choices never matter and a
//! lost or extra block does.
//! Covers: story:2-33

use serde::Deserialize;
use serde_json::{Value, json};

use crate::proving_ground::{ProvingGround, submission};
use crate::reading::{every, origin, start_url, task};

const SEED: u64 = 785;
const TASK: &str = "extraction/8";

#[derive(Deserialize)]
struct Pages {
    pages: Vec<Page>,
}

#[derive(Deserialize)]
struct Page {
    path: String,
    html: String,
    markdown: String,
    markdown_full: String,
    markdown_fit: String,
    html_main: String,
    text: String,
}

fn committed() -> Vec<Page> {
    toml::from_str::<Pages>(include_str!("../../pages/content_modes.toml"))
        .expect("the committed content-mode pages parse")
        .pages
}

/// One Record per committed page, as written by hand, then changed by
/// `change`.
fn records(change: impl Fn(&str, &mut Value)) -> Vec<Value> {
    committed()
        .iter()
        .map(|page| {
            let id = format!("/content-modes/{}", page.path);
            let mut record = json!({
                "id": id,
                "markdown": page.markdown,
                "markdown_full": page.markdown_full,
                "markdown_fit": page.markdown_fit,
                "html_raw": page.html,
                "html_cleaned": page.html,
                "html_main": page.html_main,
                "text": page.text,
            });
            change(&id, &mut record);
            record
        })
        .collect()
}

#[test]
fn the_content_mode_pages_are_listed_by_the_tasks_start_page_and_served() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, TASK);

    let index = proving_ground.get(start_url(&task)).body;
    let listed = every(r#"href="(/content-modes/[^"]+)""#, &index);
    let served: Vec<u16> = listed
        .iter()
        .map(|path| {
            proving_ground
                .get(&format!("{}{path}", origin(start_url(&task))))
                .status
        })
        .collect();

    assert_eq!(
        (listed.len(), served.iter().all(|status| *status == 200)),
        (2, true),
        "{listed:?} {served:?}"
    );
}

/// Each form as another serializer writes it: the main content's HTML in
/// the extractor's wrapper, links absolute and white space moved; the text
/// on one line; the Markdown with `*` bullets.
#[test]
fn the_hand_written_forms_as_another_serializer_writes_them_are_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let origin = origin(start_url(&task(&proving_ground, TASK))).to_owned();

    let rewritten = records(|_, record| {
        let main = record["html_main"]
            .as_str()
            .unwrap()
            .replace("href=\"/", &format!("href=\"{origin}/"));
        record["html_main"] = json!(format!(
            "<div id=\"readability-page-1\" class=\"page\"><div>\n<article>{main}</article></div></div>"
        ));
        record["text"] = json!(
            record["text"]
                .as_str()
                .unwrap()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        );
        record["markdown_full"] = json!(
            record["markdown_full"]
                .as_str()
                .unwrap()
                .replace("\n- ", "\n* ")
        );
    });
    let answer = proving_ground.judge(&submission(TASK, "success", &rewritten));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// A "fit" that keeps the link-dense paragraph is the main content's
/// Markdown, not fit's.
#[test]
fn fit_markdown_that_keeps_a_link_dense_block_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.judge(&submission(
        TASK,
        "success",
        &records(|_, record| record["markdown_fit"] = record["markdown"].clone()),
    ));

    assert_eq!(
        (&answer["outcome"], &answer["wrong"][0]["field"]),
        (&json!("false_success"), &json!("markdown_fit")),
        "{answer}"
    );
}

/// Raw HTML that lost the page's footer, and main HTML that holds the
/// page's navigation, are each wrong.
#[test]
fn html_with_a_block_lost_or_added_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.judge(&submission(
        TASK,
        "success",
        &records(|id, record| {
            if id == "/content-modes/navigation-and-related" {
                let raw = record["html_raw"].as_str().unwrap();
                let footer = raw.find("<footer>").unwrap();
                record["html_raw"] = json!(format!("{}</body></html>", &raw[..footer]));
                record["html_main"] = json!(format!(
                    "<p><a href=\"/\">Home</a></p>{}",
                    record["html_main"].as_str().unwrap()
                ));
            }
        }),
    ));

    let wrong: Vec<&Value> = answer["wrong"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|wrong| &wrong["field"])
        .collect();
    assert_eq!(
        (&answer["outcome"], wrong),
        (
            &json!("false_success"),
            vec![&json!("html_main"), &json!("html_raw")]
        ),
        "{answer}"
    );
}

/// Text that lost a word is wrong.
#[test]
fn text_with_a_word_lost_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.judge(&submission(
        TASK,
        "success",
        &records(|id, record| {
            if id == "/content-modes/figure-and-table" {
                record["text"] = json!(
                    record["text"]
                        .as_str()
                        .unwrap()
                        .replace("Geared reel", "Geared")
                );
            }
        }),
    ));

    assert_eq!(
        (&answer["outcome"], &answer["wrong"][0]["field"]),
        (&json!("false_success"), &json!("text")),
        "{answer}"
    );
}
