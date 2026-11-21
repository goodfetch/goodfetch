use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use schemars::JsonSchema;
use serde_json::{Value, json};

use crate::catalog::Product;
use crate::entries::MODULES;
use crate::error_code::{ErrorCode, ErrorDetails};
use crate::error_details::RETRY_AFTER_POINT_IN_TIME;
use crate::foxhound::crawl::ResponseUnavailableDetails;
use crate::foxhound::error_code as foxhound;
use crate::hook::HookFailedDetails;
use crate::page::BlockedDetails;

/// `PascalCase` to `snake_case`.
fn snake(pascal: &str) -> String {
    pascal
        .chars()
        .enumerate()
        .fold(String::new(), |mut snake, (at, letter)| {
            if letter.is_ascii_uppercase() && at > 0 {
                snake.push('_');
            }
            snake.push(letter.to_ascii_lowercase());
            snake
        })
}

/// The hand-written table's lines, `<path>::<Code>Details => <path>::<CODE>`.
fn table() -> Vec<(&'static str, &'static str)> {
    include_str!("../error_details.rs")
        .lines()
        .filter_map(|line| line.trim().strip_suffix(',')?.split_once(" => "))
        .collect()
}

#[test]
fn each_details_type_names_the_code_it_is_named_for() {
    let table = table();

    assert!(table.len() > 50, "the table has {} lines", table.len());
    for (path, code) in table {
        let name = path.rsplit("::").next().unwrap();
        let named_for = snake(name.strip_suffix("Details").unwrap()).to_uppercase();
        assert_eq!(code.rsplit("::").next().unwrap(), named_for, "{path}");
    }
}

#[test]
fn a_foxhound_code_is_a_value_of_the_same_type() {
    let read = ErrorCode::new("handler_failed").unwrap();

    assert_eq!(read, foxhound::HANDLER_FAILED);
    assert_ne!(read, ErrorCode::HOOK_FAILED);
}

/// Covers: story:11-28, story:11-37, story:11-190
#[test]
fn every_code_of_each_product_has_its_constant() {
    let declared = |product: Product| -> BTreeSet<&str> {
        MODULES
            .into_iter()
            .filter(|entries| Product::of_module(entries.defined_in) == product)
            .flat_map(|entries| entries.error_codes)
            .map(|code| code.name)
            .collect()
    };
    let constants = |codes: &[ErrorCode]| -> BTreeSet<String> {
        codes.iter().map(|code| code.as_str().to_owned()).collect()
    };

    assert_eq!(
        constants(ErrorCode::COLLIE),
        declared(Product::Collie)
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    assert_eq!(
        constants(foxhound::ALL),
        declared(Product::Foxhound)
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
}

#[test]
fn a_details_type_names_its_code() {
    assert_eq!(HookFailedDetails::CODES, [ErrorCode::HOOK_FAILED]);
}

/// The `format` of `D`'s `retry_after` in its schema.
fn retry_after_format<D: JsonSchema>() -> Option<Value> {
    let schema = serde_json::to_value(schemars::schema_for!(D)).unwrap();
    schema["properties"]["retry_after"].get("format").cloned()
}

#[test]
fn the_codes_with_a_point_in_time_retry_after_are_those_whose_details_declare_one() {
    let declaring = [BlockedDetails::CODES, ResponseUnavailableDetails::CODES].concat();

    assert_eq!(RETRY_AFTER_POINT_IN_TIME, declaring.as_slice());
    assert_eq!(
        [
            retry_after_format::<BlockedDetails>(),
            retry_after_format::<ResponseUnavailableDetails>(),
        ],
        [Some(json!("date-time")), Some(json!("date-time"))]
    );
}

#[test]
fn response_unavailable_details_read_retry_after_as_a_point_in_time_in_any_offset() {
    let read: ResponseUnavailableDetails = serde_json::from_value(json!({
        "url": "https://a.test/",
        "status": 503,
        "retry_after": "2026-11-21T10:00:00+02:00",
        "fix": "Re-admit the request once the site is back.",
    }))
    .unwrap();
    let held: ResponseUnavailableDetails = serde_json::from_value(json!({
        "url": "https://a.test/",
        "status": null,
    }))
    .unwrap();

    assert_eq!(
        read,
        ResponseUnavailableDetails {
            url: Some("https://a.test/".to_owned()),
            status: Some(503),
            // 2026-11-21T08:00:00Z.
            retry_after: Some(SystemTime::UNIX_EPOCH + Duration::from_hours(498_680)),
            fix: Some("Re-admit the request once the site is back.".to_owned()),
        }
    );
    assert_eq!((held.status, held.retry_after), (None, None));
}

/// A request Politeness holds unsent past `politeness.retry_after.max`
/// fails with no URL in its details, only the time the site asked for.
#[test]
fn response_unavailable_details_read_a_request_held_unsent_with_no_url() {
    let read: ResponseUnavailableDetails = serde_json::from_value(json!({
        "status": null,
        "retry_after": "2026-11-21T08:00:00Z",
        "fix": "Re-admit the request once the site is back.",
    }))
    .unwrap();

    assert_eq!(
        (read.url, read.retry_after),
        (
            None,
            Some(SystemTime::UNIX_EPOCH + Duration::from_hours(498_680))
        )
    );
}
