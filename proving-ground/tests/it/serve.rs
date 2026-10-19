//! `serve`'s first output line: one JSON object with the bound address, the
//! version and the seed, so every test and Journey variant starts its own
//! Proving ground on port 0 (ADRs 0062 and 0063).
//! Covers: story:2-80

use regex::Regex;
use serde_json::Value;

use crate::proving_ground::ProvingGround;

#[test]
fn serve_on_port_zero_first_prints_its_address_version_and_seed() {
    let proving_ground = ProvingGround::start(u64::MAX);

    let line: Value = serde_json::from_str(proving_ground.line()).expect("the line is JSON");

    let object = line.as_object().expect("the line is one JSON object");
    assert_eq!(
        object.keys().collect::<Vec<_>>(),
        ["address", "seed", "version"]
    );
    assert!(
        Regex::new(r"^127\.0\.0\.1:[1-9][0-9]*$")
            .unwrap()
            .is_match(line["address"].as_str().unwrap()),
        "{line}"
    );
    assert_eq!(line["seed"].as_u64(), Some(u64::MAX));
    assert!(
        Regex::new(r"^0\.0\.0\+[0-9a-f]{40}$")
            .unwrap()
            .is_match(line["version"].as_str().unwrap()),
        "{line}"
    );
}

#[test]
fn serve_answers_on_the_address_it_reported() {
    let proving_ground = ProvingGround::start(1);

    let tasks = proving_ground.tasks();

    assert_eq!(tasks["proving_ground"]["seed"], 1);
}
