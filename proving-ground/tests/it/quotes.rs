//! The `quotes` family (#1535): a dependent form for golden scenario 01. Choosing
//! an author posts the form back and reloads it with that author's tags, a
//! tag is valid only for the author whose tags were loaded, and Search
//! rejects the typed confirmation the first time with a message saying what
//! to type. The oracle verifies only the quotes the accepted search shows.
//!
//! The known-correct results come from posting the form over plain HTTP,
//! reading the rejection's message and typing it again.

use std::collections::BTreeSet;

use regex::Regex;
use rstest::rstest;
use serde_json::{Value, json};

use crate::proving_ground::{Answer, ProvingGround, submission};

const SEED: u64 = 1535;
const HOST: &str = "1.quotes.test";
/// The author and tag the task names.
const AUTHOR: &str = "Cleo Marsh";
const TAG: &str = "inspirational";

fn captures(text: &str, pattern: &str) -> Vec<String> {
    Regex::new(pattern)
        .expect("a valid pattern")
        .captures_iter(text)
        .map(|found| found[1].to_owned())
        .collect()
}

fn capture(text: &str, pattern: &str) -> String {
    captures(text, pattern)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("{pattern} in {text}"))
}

/// The form's hidden state, which each postback mints anew.
fn state(page: &str) -> String {
    capture(
        page,
        r#"<input type="hidden" name="state" value="([^"]+)">"#,
    )
}

/// The values a select offers, its empty placeholder aside.
fn options(page: &str, select: &str) -> Vec<String> {
    let list = capture(
        page,
        &format!(r#"(?s)<select name="{select}"[^>]*>(.*?)</select>"#),
    );
    captures(&list, r#"<option value="([^"]+)""#)
}

fn search_page(proving_ground: &ProvingGround) -> Answer {
    proving_ground.get(&proving_ground.url(HOST, "/search"))
}

/// Chooses `author`, as the author list's `onchange` posts the form back.
fn choose_author(proving_ground: &ProvingGround, author: &str) -> Answer {
    let page = search_page(proving_ground);
    proving_ground.post_form(
        &proving_ground.url(HOST, "/search"),
        &[("state", &state(&page.body)), ("author", author)],
        None,
    )
}

/// Presses Search on a form whose state is `state`.
fn search(
    proving_ground: &ProvingGround,
    state: &str,
    (author, tag, confirm): (&str, &str, &str),
) -> Answer {
    proving_ground.post_form(
        &proving_ground.url(HOST, "/filter"),
        &[
            ("state", state),
            ("author", author),
            ("tag", tag),
            ("confirm", confirm),
        ],
        None,
    )
}

fn error(page: &str) -> String {
    capture(page, r#"<p class="error" role="alert">([^<]+)</p>"#)
}

/// The quotes a results page shows, as Records.
fn quotes(page: &str) -> Vec<Value> {
    Regex::new(
        r#"<li class="quote" data-id="([^"]+)"><span class="text">([^<]+)</span> <span class="author">([^<]+)</span> <span class="tag">([^<]+)</span></li>"#,
    )
    .expect("a valid pattern")
    .captures_iter(page)
    .map(|quote| {
        json!({ "id": &quote[1], "text": &quote[2], "author": &quote[3], "tag": &quote[4] })
    })
    .collect()
}

/// Covers: story:2-33
#[test]
fn the_tag_list_is_empty_until_an_author_is_chosen_and_then_offers_only_its_tags() {
    let proving_ground = ProvingGround::start(SEED);
    let page = search_page(&proving_ground);
    let authors = options(&page.body, "author");

    let tags: Vec<Vec<String>> = authors
        .iter()
        .map(|author| options(&choose_author(&proving_ground, author).body, "tag"))
        .collect();

    assert_eq!(page.status, 200, "{}", page.body);
    assert!(
        page.body.contains(r#"<select name="tag" disabled>"#),
        "{}",
        page.body
    );
    assert_eq!(options(&page.body, "tag"), Vec::<String>::new());
    assert!(authors.iter().any(|author| author == AUTHOR), "{authors:?}");
    assert!(tags.iter().all(|offered| offered.len() == 4), "{tags:?}");
    assert!(
        tags[authors.iter().position(|author| author == AUTHOR).unwrap()]
            .iter()
            .any(|tag| tag == TAG),
        "{tags:?}"
    );
    assert!(
        tags.iter().collect::<BTreeSet<_>>().len() > 1,
        "every author offers the same tags: {tags:?}"
    );
}

/// Covers: story:2-33
#[test]
fn a_tag_the_chosen_author_is_not_offered_is_refused() {
    let proving_ground = ProvingGround::start(SEED);
    let authors = options(&search_page(&proving_ground).body, "author");
    let offered: Vec<(String, Answer)> = authors
        .into_iter()
        .map(|author| {
            let page = choose_author(&proving_ground, &author);
            (author, page)
        })
        .collect();
    let every_tag: BTreeSet<String> = offered
        .iter()
        .flat_map(|(_, page)| options(&page.body, "tag"))
        .collect();
    let (author, page, tag) = offered
        .iter()
        .find_map(|(author, page)| {
            let own = options(&page.body, "tag");
            every_tag
                .iter()
                .find(|tag| !own.contains(tag))
                .map(|tag| (author, page, tag))
        })
        .expect("an author without one of the tags");

    let refused = search(&proving_ground, &state(&page.body), (author, tag, tag));

    assert_eq!(refused.status, 400, "{}", refused.body);
    assert_eq!(
        error(&refused.body),
        format!("The tag {tag} is not offered for {author}.")
    );
    assert_eq!(quotes(&refused.body), Vec::<Value>::new());
}

/// Covers: story:2-33
#[test]
fn a_search_whose_author_changed_without_reloading_its_tags_is_refused() {
    let proving_ground = ProvingGround::start(SEED);
    let authors = options(&search_page(&proving_ground).body, "author");
    let other = authors
        .iter()
        .find(|author| *author != AUTHOR)
        .expect("another author");
    let other_tag = options(&choose_author(&proving_ground, other).body, "tag")[0].clone();
    let loaded = choose_author(&proving_ground, AUTHOR);

    let refused = search(
        &proving_ground,
        &state(&loaded.body),
        (other, &other_tag, &other_tag),
    );

    assert_eq!(refused.status, 400, "{}", refused.body);
    assert_eq!(
        error(&refused.body),
        "The author changed since its tags were loaded: choose the author again."
    );
}

/// Covers: story:2-33
#[test]
fn the_first_confirmation_is_rejected_with_what_to_type_and_a_wrong_one_again() {
    let proving_ground = ProvingGround::start(SEED);
    let loaded = choose_author(&proving_ground, AUTHOR);
    let first = search(&proving_ground, &state(&loaded.body), (AUTHOR, TAG, TAG));
    let asked = capture(&error(&first.body), r"Type ([^ ]+) to confirm the tag\.");

    let wrong = search(&proving_ground, &state(&first.body), (AUTHOR, TAG, TAG));

    assert_eq!(first.status, 422, "{}", first.body);
    assert_eq!(quotes(&first.body), Vec::<Value>::new());
    assert!(
        options(&first.body, "tag").contains(&TAG.to_owned()),
        "{}",
        first.body
    );
    assert_eq!(wrong.status, 422, "{}", wrong.body);
    assert_eq!(
        error(&wrong.body),
        format!("Type {asked} to confirm the tag.")
    );
    assert_eq!(quotes(&wrong.body), Vec::<Value>::new());
}

/// Covers: story:2-4, story:2-33
#[rstest]
#[case::read_the_rejection_and_typed_again(true, "verified")]
#[case::typed_once_and_moved_on(false, "false_success")]
fn only_a_program_that_typed_again_is_verified(#[case] types_again: bool, #[case] outcome: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let task = proving_ground.task("quotes/1");
    let loaded = choose_author(&proving_ground, AUTHOR);
    let mut results = search(&proving_ground, &state(&loaded.body), (AUTHOR, TAG, TAG));
    if types_again {
        let asked = capture(&error(&results.body), r"Type ([^ ]+) to confirm the tag\.");
        results = search(
            &proving_ground,
            &state(&results.body),
            (AUTHOR, TAG, &asked),
        );
        assert_eq!(results.status, 200, "{}", results.body);
    }
    let records = quotes(&results.body);

    let answer = proving_ground.judge(&submission("quotes/1", "success", &records));

    assert_eq!(task["kind"], "crawl", "{task}");
    assert_eq!(answer["outcome"], outcome, "{answer}");
}

/// Covers: story:2-33
#[test]
fn each_seed_has_its_own_quotes_and_confirmation() {
    let read = |seed| {
        let proving_ground = ProvingGround::start(seed);
        let loaded = choose_author(&proving_ground, AUTHOR);
        let first = search(&proving_ground, &state(&loaded.body), (AUTHOR, TAG, TAG));
        let asked = capture(&error(&first.body), r"Type ([^ ]+) to confirm the tag\.");
        let results = search(&proving_ground, &state(&first.body), (AUTHOR, TAG, &asked));
        (asked, quotes(&results.body))
    };

    let (first, second) = (read(SEED), read(SEED + 1));

    assert_ne!(first.0, second.0);
    assert_ne!(first.1, second.1);
}
