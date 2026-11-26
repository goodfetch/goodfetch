use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Holder {
    #[serde(with = "super")]
    took: Duration,
}

#[test]
fn a_duration_crosses_the_wire_as_whole_milliseconds() {
    let read: Holder = serde_json::from_value(json!({ "took": 1_500 })).unwrap();

    assert_eq!(read.took, Duration::from_millis(1_500));
    assert_eq!(
        serde_json::to_value(&Holder {
            took: Duration::from_micros(1_500_999),
        })
        .unwrap(),
        json!({ "took": 1_500 }),
        "a part of a millisecond is dropped, as the wire counts whole ones"
    );
}

#[test]
fn a_duration_past_what_the_wire_counts_is_refused() {
    let written = serde_json::to_value(Holder {
        took: Duration::MAX,
    });

    assert!(written.is_err(), "{written:?}");
}
