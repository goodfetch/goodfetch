//! The `extraction` family's Markdown pages (task `extraction/7`, ADR
//! 0007): the lab's 30 hand-written fixtures and a page with no main
//! content, each judged by its Markdown's structure, so a converter's
//! choice of syntax never matters and a lost heading, row or link does.
//! Covers: story:2-33

use serde::Deserialize;
use serde_json::{Value, json};

use crate::proving_ground::{ProvingGround, submission};
use crate::reading::{every, origin, start_url, task};

const SEED: u64 = 783;

#[derive(Deserialize)]
struct Pages {
    pages: Vec<Page>,
}

#[derive(Deserialize)]
struct Page {
    path: String,
    markdown: String,
}

/// The committed pages and their hand-written Markdown.
fn committed() -> Vec<Page> {
    toml::from_str::<Pages>(include_str!("../../pages/markdown.toml"))
        .expect("the committed Markdown pages parse")
        .pages
}

/// One Record per committed page, its Markdown rewritten by `rewrite`.
fn records(rewrite: impl Fn(&str, &str) -> String) -> Vec<Value> {
    committed()
        .iter()
        .map(|page| {
            let id = format!("/markdown/{}", page.path);
            json!({ "id": id, "markdown": rewrite(&id, &page.markdown) })
        })
        .collect()
}

/// The same Markdown in another converter's syntax: `*` bullets, a setext
/// first-level heading, tilde fences, every other heading but a sixth-level
/// one a level deeper, and each relative link absolute on the page's own
/// origin. A code block's content stays as it is.
fn other_syntax(origin: &str, markdown: &str) -> String {
    let mut written = Vec::new();
    let mut fence: Option<String> = None;
    for line in markdown.lines() {
        if let Some(open) = &fence {
            if line == open {
                written.push("~".repeat(open.len()));
                fence = None;
            } else {
                written.push(line.to_owned());
            }
            continue;
        }
        if line.starts_with("```") {
            let marker: String = line.chars().take_while(|c| *c == '`').collect();
            written.push(format!(
                "{}{}",
                "~".repeat(marker.len()),
                &line[marker.len()..]
            ));
            fence = Some(marker);
            continue;
        }
        let line = line.replace("](/", &format!("]({origin}/"));
        let line = match line.strip_prefix("- ") {
            Some(item) => format!("* {item}"),
            None => line,
        };
        if let Some(title) = line.strip_prefix("# ") {
            written.push(title.to_owned());
            written.push("=".repeat(title.chars().count().max(3)));
        } else if line.starts_with('#') && !line.starts_with("######") {
            written.push(format!("#{line}"));
        } else {
            written.push(line);
        }
    }
    written.join("\n") + "\n"
}

#[test]
fn the_markdown_pages_are_listed_by_the_tasks_start_page_and_served() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "extraction/7");

    let index = proving_ground.get(start_url(&task)).body;
    let listed = every(r#"href="(/markdown/[^"]+)""#, &index);
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
        (31, true),
        "{listed:?} {served:?}"
    );
}

#[test]
fn the_hand_written_markdown_in_another_syntax_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "extraction/7");
    let origin = origin(start_url(&task)).to_owned();

    let answer = proving_ground.judge(&submission(
        "extraction/7",
        "success",
        &records(|_, markdown| other_syntax(&origin, markdown)),
    ));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_page_left_out_is_incomplete() {
    let proving_ground = ProvingGround::start(SEED);
    let mut records = records(|_, markdown| markdown.to_owned());
    records.retain(|record| record["id"] != "/markdown/tables/simple");

    let answer = proving_ground.judge(&submission("extraction/7", "failure", &records));

    assert_eq!(
        (&answer["outcome"], &answer["missing"]),
        (&json!("incomplete"), &json!(["/markdown/tables/simple"])),
        "{answer}"
    );
}

/// A converter that writes every heading as a paragraph keeps the text and
/// loses the structure.
#[test]
fn headings_written_as_paragraphs_are_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.judge(&submission(
        "extraction/7",
        "success",
        &records(|_, markdown| {
            markdown
                .lines()
                .map(|line| line.trim_start_matches('#').trim_start())
                .collect::<Vec<_>>()
                .join("\n")
        }),
    ));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
}

/// A page with no main content answers empty Markdown, never its
/// navigation in its place (pitfall D02).
#[test]
fn the_navigation_of_a_page_with_no_main_content_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.judge(&submission(
        "extraction/7",
        "success",
        &records(|id, markdown| {
            if id == "/markdown/none/navigation-only" {
                "[Pages](/markdown) [Products](/products)\n".to_owned()
            } else {
                markdown.to_owned()
            }
        }),
    ));

    assert_eq!(
        (&answer["outcome"], &answer["wrong"][0]["id"]),
        (
            &json!("false_success"),
            &json!("/markdown/none/navigation-only")
        ),
        "{answer}"
    );
}
