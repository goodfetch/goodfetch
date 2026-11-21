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

#[test]
fn the_task_list_names_the_tasks_of_every_family() {
    let proving_ground = ProvingGround::start(158);

    let tasks = proving_ground.tasks();

    assert_eq!(tasks["proving_ground"]["seed"], 158, "{tasks}");
    let outside_their_family: Vec<&Value> = tasks["tasks"]
        .as_array()
        .expect("`tasks` is an array")
        .iter()
        .filter(|task| task["id"].as_str().unwrap().split('/').next() != task["family"].as_str())
        .collect();
    assert!(outside_their_family.is_empty(), "{outside_their_family:?}");
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

/// `tasks --seed N` prints the list `serve` serves for that seed, with no
/// port, since it binds none, so `xtask journeys tasks` can commit it
/// (ADR 0063, rule 3).
#[test]
fn tasks_prints_the_served_list_with_no_port() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_goodfetch-proving-ground"))
        .args(["tasks", "--seed", "158"])
        .env_clear()
        .output()
        .expect("the Proving ground runs");
    let served = ProvingGround::start(158).tasks();

    assert!(output.status.success(), "{output:?}");
    let printed: Value = serde_json::from_slice(&output.stdout).expect("it prints JSON");
    assert_eq!(printed["proving_ground"], served["proving_ground"]);
    assert_eq!(each(&printed, "id"), each(&served, "id"));
    assert_eq!(each(&printed, "digest"), each(&served, "digest"));
    assert_eq!(
        printed["tasks"][0]["start_url"],
        "http://1.catalog.test/catalog/1"
    );
    // Every family's start URL, on its own host name and no port.
    let portless = Regex::new(r"^https?://[^/:]+(/|$)").unwrap();
    assert!(
        each(&printed, "start_url")
            .iter()
            .all(|url| portless.is_match(url.as_str().unwrap())),
        "{printed}"
    );
}
