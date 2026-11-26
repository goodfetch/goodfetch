use std::time::Duration;

use goodfetch_protocol::page::functions::{Declared, LocatorScrollCollectParams};
use rstest::rstest;
use serde_json::json;

use super::{Bounds, Kept, declared};
use crate::page_functions::Collected;

/// A step's read, as `collect.js` answers it, of the rows `keys` in order.
fn read(keys: &[&str]) -> Collected {
    serde_json::from_value(json!({
        "rows": keys.iter().map(|key| json!({ "key": key, "value": { "name": key } })).collect::<Vec<_>>(),
        "matched": keys.len(),
        "declared": null,
    }))
    .expect("invariant: the test's read is collect.js's shape")
}

/// A key seen again keeps its first value, and rows stay in the order
/// their key was first seen, across steps whose windows overlap.
#[test]
fn a_key_seen_again_keeps_its_first_row() {
    let mut kept = Kept::default();

    let first = kept.keep(read(&["a", "b", "c"]), 100);
    let second = kept.keep(read(&["b", "c", "d"]), 100);

    assert_eq!((first, second), (3, 1));
    assert_eq!(
        kept.rows,
        vec![
            json!({ "name": "a" }),
            json!({ "name": "b" }),
            json!({ "name": "c" }),
            json!({ "name": "d" }),
        ]
    );
    assert_eq!(kept.observed(), "4 rows kept, the last key `d`");
}

/// It keeps no more than `max_rows`, even when one step reads more.
#[test]
fn it_keeps_no_more_than_max_rows() {
    let mut kept = Kept::default();

    kept.keep(read(&["a", "b", "c", "d"]), 3);

    assert_eq!(kept.rows.len(), 3);
    assert!(kept.full(3));
    assert_eq!(kept.observed(), "3 rows kept, the last key `c`");
}

/// Nothing kept says so.
#[test]
fn nothing_kept_says_so() {
    assert_eq!(Kept::default().observed(), "no rows kept");
}

/// `-1` declares the rows unknown; another negative declares none.
#[rstest]
#[case::count(Some(1000), Some(Declared::Rows(1000)))]
#[case::zero(Some(0), Some(Declared::Rows(0)))]
#[case::unknown(Some(-1), Some(Declared::Unknown))]
#[case::other_negative(Some(-2), None)]
#[case::none(None, None)]
fn what_a_list_declares(#[case] read: Option<i64>, #[case] expected: Option<Declared>) {
    assert_eq!(declared(read), expected);
}

/// The call's params, with `bounds` beside its locator and function.
fn params(bounds: serde_json::Value) -> LocatorScrollCollectParams {
    let mut params = json!({
        "locator": { "page_id": "page-1", "selector": "[role=row]" },
        "function": "(element) => element.textContent",
    });
    if let (Some(params), serde_json::Value::Object(bounds)) = (params.as_object_mut(), bounds) {
        params.extend(bounds);
    }
    serde_json::from_value(params).expect("invariant: the test's params are the wire's shape")
}

/// Bounds the call leaves out take their defaults; those it names are kept.
#[test]
fn bounds_default_where_the_call_names_none() {
    let defaults = Bounds::of(&params(json!({}))).ok();
    let named = Bounds::of(&params(
        json!({ "max_rows": 50, "idle_steps": 1, "step_wait": 250 }),
    ))
    .ok();

    assert_eq!(
        defaults,
        Some(Bounds {
            max_rows: 10_000,
            idle_steps: 3,
            step_wait: Duration::from_millis(2_000),
        })
    );
    assert_eq!(
        named,
        Some(Bounds {
            max_rows: 50,
            idle_steps: 1,
            step_wait: Duration::from_millis(250),
        })
    );
}

/// A bound of 0 is refused as `invalid_params`.
#[rstest]
#[case::max_rows(json!({ "max_rows": 0 }))]
#[case::idle_steps(json!({ "idle_steps": 0 }))]
#[case::step_wait(json!({ "step_wait": 0 }))]
fn a_bound_of_zero_is_refused(#[case] bounds: serde_json::Value) {
    let refused = Bounds::of(&params(bounds)).map_err(|error| error.code().to_owned());

    assert_eq!(refused, Err("invalid_params".to_owned()));
}
