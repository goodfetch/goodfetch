//! The `fields` family (pitfall D07): listings posted over time, each newer
//! batch with a field the earlier ones lack, so a crawl that fixes its
//! columns by its first Record drops the later fields, and the oracle names
//! each value it dropped.
//! Covers: story:2-33

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::crawler::{Crawler, Fetched, field};
use crate::proving_ground::{ProvingGround, submission};

const SEED: u64 = 1538;
const TASK: &str = "fields/1";

/// Each listing's Record: its `id` and every field its page shows.
fn records(pages: &[Fetched]) -> Vec<Value> {
    let shown = Regex::new(r#"<dd class="([a-z]+)">([^<]*)</dd>"#).expect("a valid pattern");
    pages
        .iter()
        .filter_map(|page| {
            let body = &page.answer.body;
            let mut record = Map::new();
            record.insert(
                "id".to_owned(),
                field(body, r#"<article class="entry" data-id="([^"]+)""#)?.into(),
            );
            for captures in shown.captures_iter(body) {
                record.insert(captures[1].to_owned(), captures[2].into());
            }
            Some(Value::Object(record))
        })
        .collect()
}

/// The Records as an export whose columns are its first Record's fields
/// writes them.
fn columns_of_the_first(records: &[Value]) -> Vec<Value> {
    let columns: Vec<String> = records[0]
        .as_object()
        .expect("a Record is an object")
        .keys()
        .cloned()
        .collect();
    records
        .iter()
        .map(|record| {
            Value::Object(
                columns
                    .iter()
                    .filter_map(|column| Some((column.clone(), record.get(column)?.clone())))
                    .collect(),
            )
        })
        .collect()
}

fn crawl(proving_ground: &ProvingGround) -> Vec<Value> {
    let start = proving_ground.task(TASK)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned();
    records(&Crawler::new(proving_ground).crawl(&start))
}

#[test]
fn a_crawl_that_keeps_every_field_of_every_record_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let records = crawl(&proving_ground);

    let answer = proving_ground.judge(&submission(TASK, "success", &records));

    assert_eq!(answer["outcome"], "verified", "{answer}");
    assert_eq!(records.len(), 24, "{answer}");
}

#[test]
fn a_later_record_carries_fields_the_first_lacks() {
    let proving_ground = ProvingGround::start(SEED);
    let records = crawl(&proving_ground);

    let width = |record: &Value| record.as_object().unwrap().len();

    assert_eq!(width(&records[0]), 2, "{}", records[0]);
    assert_eq!(width(&records[23]), 4, "{}", records[23]);
}

#[test]
fn a_crawl_whose_columns_are_its_first_records_is_a_false_success_naming_each_dropped_field() {
    let proving_ground = ProvingGround::start(SEED);
    let records = columns_of_the_first(&crawl(&proving_ground));

    let answer = proving_ground.judge(&submission(TASK, "success", &records));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["missing"], json!([]), "{answer}");
    let wrong = answer["wrong"].as_array().unwrap();
    let dropped = |name: &str| {
        wrong
            .iter()
            .filter(|wrong| wrong["field"] == name && wrong["submitted"].is_null())
            .count()
    };
    assert_eq!(dropped("price"), 16, "{answer}");
    assert_eq!(dropped("rating"), 8, "{answer}");
    assert_eq!(wrong.len(), 24, "{answer}");
}
