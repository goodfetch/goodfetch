//! The task list (ADR 0063): each task's ID is `<family>/<ordinal>`, the same
//! for every seed, and its digest pins the task's entry and expected result
//! whatever port `serve` bound.
//! Covers: story:2-79

use regex::Regex;
use serde_json::{Value, json};

use crate::proving_ground::ProvingGround;

/// Each task's `field`, in list order.
fn each(tasks: &Value, field: &str) -> Vec<Value> {
    tasks["tasks"]
        .as_array()
        .expect("`tasks` is an array")
        .iter()
        .map(|task| task[field].clone())
        .collect()
}

#[test]
fn the_task_list_names_the_tasks_of_every_family() {
    let proving_ground = ProvingGround::start(158);

    let tasks = proving_ground.tasks();

    assert_eq!(tasks["proving_ground"]["seed"], 158, "{tasks}");
    let listed: Vec<Value> = tasks["tasks"]
        .as_array()
        .expect("`tasks` is an array")
        .iter()
        .map(|task| json!([task["id"], task["family"], task["kind"]]))
        .collect();
    assert_eq!(
        listed,
        [
            json!(["catalog/1", "catalog", "crawl"]),
            json!(["catalog/2", "catalog", "extract"]),
            json!(["catalog/3", "catalog", "submit"]),
            json!(["agent/1", "agent", "submit"]),
            json!(["agent/2", "agent", "extract"]),
            json!(["agent/3", "agent", "extract"]),
            json!(["agent/4", "agent", "extract"]),
            json!(["agent/5", "agent", "extract"]),
            json!(["agent/6", "agent", "extract"]),
            json!(["agent/7", "agent", "extract"]),
            json!(["agent/8", "agent", "extract"]),
            json!(["agent/9", "agent", "extract"]),
        ]
    );
    assert_eq!(
        tasks["tasks"][0]["start_url"],
        format!("http://1.catalog.test:{}/catalog/1", proving_ground.port())
    );
    let product_page = Regex::new(&format!(
        r"^http://2\.catalog\.test:{}/products/P-[0-9]{{4}}$",
        proving_ground.port()
    ))
    .unwrap();
    assert!(
        product_page.is_match(tasks["tasks"][1]["start_url"].as_str().unwrap()),
        "{tasks}"
    );
    let digest = Regex::new("^[0-9a-f]{64}$").unwrap();
    let digests = each(&tasks, "digest");
    assert!(
        digests.iter().all(|d| digest.is_match(d.as_str().unwrap())),
        "{tasks}"
    );
    let descriptions = each(&tasks, "description");
    assert!(
        descriptions.iter().all(|d| !d.as_str().unwrap().is_empty()),
        "{tasks}"
    );
}

#[test]
fn the_same_seed_on_the_same_port_lists_the_same_tasks() {
    let first = ProvingGround::start(158);
    let address = first.address().to_owned();
    let listed = first.tasks();
    drop(first);
    let second = ProvingGround::start_on(158, &address);

    let relisted = second.tasks();

    assert_eq!(listed, relisted);
}

#[test]
fn the_same_seed_on_two_ports_lists_the_same_digests() {
    let first = ProvingGround::start(158);
    let second = ProvingGround::start(158);

    let digests = (
        each(&first.tasks(), "digest"),
        each(&second.tasks(), "digest"),
    );

    assert_ne!(first.port(), second.port());
    assert_eq!(digests.0, digests.1);
}

#[test]
fn two_seeds_list_the_same_task_ids_and_kinds() {
    let first = ProvingGround::start(158);
    let second = ProvingGround::start(2);

    let tasks = (first.tasks(), second.tasks());

    assert_eq!(each(&tasks.0, "id"), each(&tasks.1, "id"));
    assert_eq!(each(&tasks.0, "kind"), each(&tasks.1, "kind"));
}

#[test]
fn another_expected_result_changes_the_digest() {
    // The crawl task's entry is the same for both seeds on one port, so only
    // its expected result (the seed's products) differs.
    let first = ProvingGround::start(158);
    let address = first.address().to_owned();
    let mut crawl = first.tasks()["tasks"][0].clone();
    drop(first);
    let second = ProvingGround::start_on(2, &address);
    let mut other = second.tasks()["tasks"][0].clone();

    let digests = (crawl["digest"].take(), other["digest"].take());

    assert_eq!(crawl, other);
    assert_ne!(digests.0, digests.1);
}
