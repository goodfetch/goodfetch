//! The `locales` family's pages that declare their language in a meta tag, in
//! a header or on their root element (ADR 0123), each with an amount that
//! reads differently in another language.
//!
//! The known-correct results come from reading each page over plain HTTP,
//! with the rule written out here once more.
//! Covers: story:2-33

use regex::Regex;
use rstest::rstest;
use serde_json::{Value, json};

use crate::proving_ground::{Answer, ProvingGround, submission};
use crate::reading::{every, first, origin, start_url, task};

const SEED: u64 = 1540;
const TASK: &str = "locales/1";

/// Where a page declares its language.
struct Declared {
    root: Option<String>,
    meta: Option<String>,
    header: Option<String>,
}

fn declared(page: &Answer) -> Declared {
    let one = |pattern| {
        Regex::new(pattern)
            .expect("a valid pattern")
            .captures(&page.body)
            .map(|found| found[1].to_owned())
    };
    Declared {
        root: one(r#"<html lang="([^"]+)""#),
        meta: one(r#"<meta http-equiv="content-language" content="([^"]+)">"#),
        header: page.header("content-language").map(str::to_owned),
    }
}

fn one_tag(tags: Option<String>) -> Option<String> {
    tags.filter(|tags| !tags.contains(','))
}

/// The language ADR 0123 gives a page.
fn right(declared: Declared) -> String {
    declared
        .root
        .or(one_tag(declared.meta))
        .or(one_tag(declared.header))
        .unwrap_or_else(|| "und".to_owned())
}

fn root_alone(declared: Declared) -> String {
    declared.root.unwrap_or_else(|| "und".to_owned())
}

fn header_first(declared: Declared) -> String {
    one_tag(declared.header)
        .or(declared.root)
        .or(one_tag(declared.meta))
        .unwrap_or_else(|| "und".to_owned())
}

fn first_of_several(declared: Declared) -> String {
    let first = |tags: String| tags.split(',').next().expect("a tag").trim().to_owned();
    declared
        .root
        .or(declared.meta.map(first))
        .or(declared.header.map(first))
        .unwrap_or_else(|| "und".to_owned())
}

/// The plain decimal an amount reads as in a language.
fn amount(shown: &str, language: &str) -> String {
    match language {
        "de" => shown.replace('.', "").replace(',', "."),
        _ => shown.replace(',', ""),
    }
}

/// The pages the index links, read in the language `language` gives each.
fn read(proving_ground: &ProvingGround, language: fn(Declared) -> String) -> Vec<Value> {
    let task = task(proving_ground, TASK);
    let index = proving_ground.get(start_url(&task));
    assert_eq!(index.status, 200, "{}", index.body);
    every(r#"<a href="([^"]+)""#, &index.body)
        .iter()
        .map(|path| {
            let page = proving_ground.get(&format!("{}{path}", origin(start_url(&task))));
            assert_eq!(page.status, 200, "{}", page.body);
            let language = language(declared(&page));
            let shown = first(r#"<span class="amount">([^<]+)</span>"#, &page.body);
            json!({
                "id": first(r#"data-id="([^"]+)""#, &page.body),
                "amount": amount(&shown, &language),
                "language": language,
            })
        })
        .collect()
}

#[rstest]
#[case::one_seed(SEED)]
#[case::another_seed(SEED + 1)]
#[case::a_third_seed(SEED + 2)]
fn pages_read_in_the_language_they_declare_are_verified(#[case] seed: u64) {
    let proving_ground = ProvingGround::start(seed);
    let records = read(&proving_ground, right);

    let answer = proving_ground.judge(&submission(TASK, "success", &records));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[rstest]
#[case::the_root_element_alone(root_alone)]
#[case::the_header_before_the_page(header_first)]
#[case::the_first_of_several_tags(first_of_several)]
fn pages_read_in_another_language_are_refuted(#[case] language: fn(Declared) -> String) {
    let proving_ground = ProvingGround::start(SEED);
    let records = read(&proving_ground, language);

    let answer = proving_ground.judge(&submission(TASK, "success", &records));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
}

#[test]
fn an_amount_read_in_the_wrong_language_is_refuted() {
    let proving_ground = ProvingGround::start(SEED);
    let mut records = read(&proving_ground, right);
    let german = records
        .iter_mut()
        .find(|record| record["language"] == "de")
        .expect("a page in German");
    let read_as_english: f64 = german["amount"]
        .as_str()
        .expect("an amount")
        .parse()
        .expect("a number");
    german["amount"] = json!(format!("{}", read_as_english / 1000.0));

    let answer = proving_ground.judge(&submission(TASK, "success", &records));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
}

#[test]
fn every_place_a_language_is_declared_in_decides_a_page() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, TASK);
    let index = proving_ground.get(start_url(&task));
    let declared: Vec<Declared> = every(r#"<a href="([^"]+)""#, &index.body)
        .iter()
        .map(|path| declared(&proving_ground.get(&format!("{}{path}", origin(start_url(&task))))))
        .collect();
    let has = |wanted: fn(&Declared) -> bool| {
        assert_eq!(
            declared.iter().filter(|page| wanted(page)).count(),
            1,
            "a page of each kind"
        );
    };
    let several = |tags: &Option<String>| tags.as_ref().is_some_and(|tags| tags.contains(','));

    assert_eq!(declared.len(), 6);
    has(|page| page.root.is_none() && page.meta.is_some() && page.header.is_none());
    has(|page| {
        page.root.is_none() && page.meta.is_none() && one_tag(page.header.clone()).is_some()
    });
    has(|page| page.root.is_some() && page.meta != page.root && page.header != page.root);
    assert_eq!(
        declared.iter().filter(|page| several(&page.meta)).count(),
        1
    );
    assert_eq!(
        declared.iter().filter(|page| several(&page.header)).count(),
        1
    );
}
