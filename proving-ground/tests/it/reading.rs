//! Reading the Extraction families' pages the way any tool would, for their
//! known-right results: the task a test judges and the pieces a page shows.

use regex::Regex;
use serde_json::Value;

use crate::proving_ground::ProvingGround;

/// The task `id` of the task list.
pub(crate) fn task(proving_ground: &ProvingGround, id: &str) -> Value {
    proving_ground.tasks()["tasks"]
        .as_array()
        .expect("`tasks` is an array")
        .iter()
        .find(|task| task["id"] == id)
        .unwrap_or_else(|| panic!("the task list has no {id}"))
        .clone()
}

/// A task's start URL.
pub(crate) fn start_url(task: &Value) -> &str {
    task["start_url"].as_str().expect("a start URL")
}

/// The scheme, host and port of a URL.
pub(crate) fn origin(url: &str) -> &str {
    let after_scheme = url.find("://").expect("an absolute URL") + "://".len();
    let end = url[after_scheme..]
        .find('/')
        .map_or(url.len(), |at| after_scheme + at);
    &url[..end]
}

/// The first group of `pattern`'s first match in `text`.
pub(crate) fn first(pattern: &str, text: &str) -> String {
    Regex::new(pattern)
        .expect("a valid pattern")
        .captures(text)
        .unwrap_or_else(|| panic!("{pattern} matches nothing in {text}"))[1]
        .to_owned()
}

/// The first group of each of `pattern`'s matches in `text`.
pub(crate) fn every(pattern: &str, text: &str) -> Vec<String> {
    Regex::new(pattern)
        .expect("a valid pattern")
        .captures_iter(text)
        .map(|found| found[1].to_owned())
        .collect()
}
