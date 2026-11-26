//! What a scroll collection answers on the wire (#807).

use rstest::rstest;
use serde_json::{Value, json};

use super::{Declared, ListEnded, ScrollCollected};

#[rstest]
#[case::a_count(Some(Declared::Rows(1000)), json!(1000))]
#[case::unknown(Some(Declared::Unknown), json!("unknown"))]
#[case::none(None, Value::Null)]
fn what_the_list_declares_is_a_count_unknown_or_null(
    #[case] declared: Option<Declared>,
    #[case] wire: Value,
) {
    let collected = ScrollCollected {
        rows: vec![json!({"title": "a"})],
        ended: ListEnded::EndOfList,
        declared,
    };

    let value = serde_json::to_value(&collected).expect("a scroll collection serializes");

    assert_eq!(
        value,
        json!({"rows": [{"title": "a"}], "ended": "end_of_list", "declared": wire})
    );
    assert_eq!(
        serde_json::from_value::<ScrollCollected>(value).expect("it reads back"),
        collected
    );
}

#[test]
fn how_a_newer_engine_ended_a_collection_is_kept_as_received() {
    let collected: ScrollCollected =
        serde_json::from_value(json!({"rows": [], "ended": "page_closed", "declared": null}))
            .expect("an unknown end reads");

    assert_eq!(collected.ended.as_str(), "page_closed");
}
