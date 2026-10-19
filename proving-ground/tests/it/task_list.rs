//! The task list (ADR 0063): each task's ID is `<family>/<ordinal>`, the same
//! for every seed, and its digest pins the task's entry and expected result
//! whatever port `serve` bound.
//! Covers: story:2-79

use regex::Regex;
use serde_json::Value;

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

/// Each task's `field`, in list order, joined by spaces.
fn joined(tasks: &Value, field: &str) -> String {
    each(tasks, field)
        .iter()
        .map(|value| value.as_str().expect("a string field"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The task list's IDs, in order.
const IDS: &str = "catalog/1 catalog/2 catalog/3 \
    adversarial/1 adversarial/2 adversarial/3 adversarial/4 \
    protection/1 protection/2";

/// Each task's kind, in the same order.
const KINDS: &str = "crawl extract submit \
    extract submit extract extract \
    crawl extract";

#[test]
fn the_task_list_names_the_tasks_of_every_family() {
    let proving_ground = ProvingGround::start(158);

    let tasks = proving_ground.tasks();

    assert_eq!(tasks["proving_ground"]["seed"], 158, "{tasks}");
    assert_eq!(joined(&tasks, "id"), IDS);
    assert_eq!(joined(&tasks, "kind"), KINDS);
    let families: Vec<&str> = IDS
        .split(' ')
        .map(|id| id.split('/').next().expect("a family"))
        .collect();
    assert_eq!(joined(&tasks, "family"), families.join(" "));
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
