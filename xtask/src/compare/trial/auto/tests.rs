use crate::compare::auto::{Read, Record};
use crate::compare::foxhound;

fn record(tier: &str, id: Option<&str>, name: Option<&str>) -> foxhound::Record {
    foxhound::Record {
        url: "https://rungs.localhost:4443/static/1".to_owned(),
        committed_at: String::new(),
        sample: false,
        title: None,
        text: None,
        id: id.map(str::to_owned),
        name: name.map(str::to_owned),
        tier: Some(tier.to_owned()),
        value: serde_json::Value::Null,
    }
}

/// A Record of Foxhound's is a page rendered when its Provenance names
/// either browser tier, never `browser_fetch`, which fetches with the
/// browser's fingerprint without rendering.
#[test]
fn foxhounds_records_count_rendered_on_a_browser_tier() {
    let rendered = |tier| Read::from(record(tier, None, None)).rendered;

    let rendered = ["browser", "light_browser", "browser_fetch", "http"].map(rendered);

    assert_eq!(rendered, [true, true, false, false]);
}

/// A Record of Foxhound's with neither field holds none of the site's
/// Records, and one with either holds one, the other field empty.
#[test]
fn foxhounds_records_hold_the_site_record_their_fields_give() {
    let records = [None, Some("static-1")].map(|id| Read::from(record("http", id, None)).records);

    assert_eq!(
        records,
        [
            Vec::new(),
            vec![Record {
                id: "static-1".to_owned(),
                name: String::new(),
            }]
        ]
    );
}
