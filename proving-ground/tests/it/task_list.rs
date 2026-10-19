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
    rendered/1 rendered/2 login/1 login/2 schedule/1 \
    portal/1 portal/2 portal/3 portal/4 portal/5 portal/6 portal/7 portal/8 portal/9 portal/10 \
    portal/11 portal/12 portal/13 \
    actionability/1 actionability/2 actionability/3 actionability/4 actionability/5 \
    actionability/6 actionability/7 actionability/8 actionability/9 actionability/10 \
    actionability/11 actionability/12 actionability/13 actionability/14 actionability/15 \
    actionability/16 actionability/17 actionability/18 actionability/19 actionability/20 \
    actionability/21 actionability/22 actionability/23 actionability/24 actionability/25 \
    actionability/26 \
    navigation/1 navigation/2 navigation/3 navigation/4 navigation/5 navigation/6 navigation/7 \
    agent/1 agent/2 agent/3 agent/4 agent/5 agent/6 agent/7 agent/8 agent/9 \
    replay/1 replay/2 \
    extraction/1 extraction/2 extraction/3 extraction/4 extraction/5 extraction/6 \
    encodings/1 encodings/2 languages/1 languages/2 files/1 files/2 files/3 \
    statuses/1 traps/1 parameters/1 limits/1 limits/2 limits/3 scale/1 scale/2";

/// Each task's kind, in the same order.
const KINDS: &str = "crawl extract submit \
    crawl extract crawl crawl watch \
    crawl crawl crawl crawl crawl crawl crawl crawl crawl crawl crawl crawl crawl \
    act act act act act act act act act act act act act act act act act act act \
    submit submit submit submit submit submit act \
    act act act act act act download \
    submit extract extract extract extract extract extract extract extract \
    extract extract \
    crawl extract crawl crawl extract crawl \
    crawl crawl crawl extract crawl extract extract \
    crawl crawl crawl crawl crawl crawl crawl crawl";

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
