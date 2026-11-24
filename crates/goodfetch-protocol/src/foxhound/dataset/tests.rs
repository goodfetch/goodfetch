use super::CrawlRequestPushParams;
use crate::foxhound::crawl::CrawlRequestHandled;

/// The fields of a Record whose program wrote them out of alphabetical
/// order.
const UNSORTED: &str = r#"{"title":"Café","price":3,"author":{"name":"Zoé","born":1802}}"#;

#[test]
fn a_records_data_keeps_its_fields_in_the_order_its_program_wrote_them() {
    let handled: CrawlRequestHandled = serde_json::from_str(&format!(
        r#"{{"records":[{{"dataset":"books","data":{UNSORTED}}}]}}"#
    ))
    .unwrap();
    let pushed: CrawlRequestPushParams = serde_json::from_str(&format!(
        r#"{{"run_id":"run-1","crawl_request_id":"r1","lease_epoch":1,"dataset":"books","records":[{UNSORTED}]}}"#
    ))
    .unwrap();

    assert_eq!(
        (handled.records[0].data.json(), pushed.records[0].json()),
        (UNSORTED, UNSORTED)
    );
}

/// A Record nested past `serde_json`'s own limit of 128, as a code-free
/// page's microdata may be, still reads as a value (issue 2394).
#[test]
fn a_record_nested_past_serde_jsons_limit_reads_as_a_value() {
    let deep = (0..400).fold(
        serde_json::json!("leaf"),
        |inner, _| serde_json::json!({ "item": [inner] }),
    );

    assert_eq!(super::RecordData::of(&deep).unwrap().value(), deep);
}
