use rstest::rstest;
use serde_json::json;

use super::{Product, ProtocolVersion};

#[rstest]
#[case::collie_contains_collie(Product::Collie, Product::Collie, true)]
#[case::collie_lacks_foxhound(Product::Collie, Product::Foxhound, false)]
#[case::foxhound_contains_collie(Product::Foxhound, Product::Collie, true)]
#[case::foxhound_contains_foxhound(Product::Foxhound, Product::Foxhound, true)]
fn a_product_contains_itself_and_foxhound_contains_collie(
    #[case] product: Product,
    #[case] other: Product,
    #[case] contains: bool,
) {
    assert_eq!(product.contains(other), contains);
}

#[rstest]
#[case::collie(Product::Collie, "collie")]
#[case::foxhound(Product::Foxhound, "foxhound")]
fn product_reads_and_writes_the_mark_the_catalog_writes(
    #[case] product: Product,
    #[case] mark: &str,
) {
    let written = serde_json::to_value(product).unwrap();

    assert_eq!(written, json!(mark));
    assert_eq!(product.as_str(), mark);
    assert_eq!(serde_json::from_value::<Product>(written).unwrap(), product);
}

#[rstest]
#[case::zero("0.1", ProtocolVersion { major: 0, minor: 1 })]
#[case::several_digits("12.304", ProtocolVersion { major: 12, minor: 304 })]
fn a_protocol_version_reads_as_major_dot_minor(
    #[case] text: &str,
    #[case] version: ProtocolVersion,
) {
    let read: ProtocolVersion = serde_json::from_value(json!(text)).unwrap();

    assert_eq!(read, version);
    assert_eq!(serde_json::to_value(read).unwrap(), json!(text));
}

#[rstest]
#[case::empty("")]
#[case::major_only("1")]
#[case::patch("1.2.3")]
#[case::leading_zero("01.2")]
#[case::sign("+1.2")]
#[case::space(" 1.2")]
#[case::too_large("70000.1")]
#[case::not_a_number("a.b")]
fn a_malformed_protocol_version_is_refused(#[case] text: &str) {
    let read = serde_json::from_value::<ProtocolVersion>(json!(text));

    assert!(read.is_err(), "{text:?} was read as {read:?}");
}

/// Covers: story:11-218
#[test]
fn the_catalog_marks_every_entry_it_keeps_to_the_sdks() {
    let unmarked: Vec<&str> = crate::entries::MODULES
        .iter()
        .flat_map(|entries| entries.operations)
        .filter(|operation| {
            super::SDK_ONLY.contains(&operation.name) && operation.reach != super::Reach::SdkOnly
        })
        .map(|operation| operation.name)
        .collect();

    assert_eq!(unmarked, Vec::<&str>::new());
}

/// A deprecation's one text (ADR 0099), which every SDK's markers and
/// warning and the CLI's carry: since when, the earliest removal, the
/// replacement when there is one, and the note.
#[rstest]
#[case::with_a_replacement(
    Some("sample.shout"),
    "Since 0.1, removed no earlier than 1.0: use `sample.shout` instead. Repeats in capitals."
)]
#[case::without_one(None, "Since 0.1, removed no earlier than 1.0: Repeats in capitals.")]
fn a_deprecation_renders_one_text(#[case] replacement: Option<&'static str>, #[case] text: &str) {
    let deprecation = super::Deprecation {
        since: ProtocolVersion { major: 0, minor: 1 },
        replacement,
        earliest_removal: ProtocolVersion { major: 1, minor: 0 },
        note: "Repeats in capitals.",
    };

    assert_eq!(deprecation.message(), text);
}

/// A deprecation reads back from the catalog file as it was written, with
/// owned text, as `xtask codegen`'s generators read it.
#[test]
fn a_deprecation_reads_back_as_the_catalog_writes_it() {
    let deprecation = super::Deprecation {
        since: ProtocolVersion { major: 0, minor: 1 },
        replacement: Some("sample.shout"),
        earliest_removal: ProtocolVersion { major: 1, minor: 0 },
        note: "Repeats in capitals.",
    };

    let written = serde_json::to_value(deprecation).unwrap();
    let read: super::Deprecation<String> = serde_json::from_value(written.clone()).unwrap();

    assert_eq!(
        written,
        json!({
            "since": "0.1",
            "replacement": "sample.shout",
            "earliest_removal": "1.0",
            "note": "Repeats in capitals.",
        })
    );
    assert_eq!(read.message(), deprecation.message());
}

/// Every deprecation of the catalog names a replacement of its own section,
/// if any, and a removal no earlier than the next major (ADR 0030).
#[test]
fn every_deprecation_names_an_entry_of_its_section_and_a_later_major() {
    let modules = crate::entries::MODULES;
    let operations: Vec<&str> = modules
        .iter()
        .flat_map(|entries| entries.operations)
        .map(|operation| operation.name)
        .collect();
    let events: Vec<&str> = modules
        .iter()
        .flat_map(|entries| entries.events)
        .map(|event| event.name)
        .collect();
    let requests: Vec<&str> = modules
        .iter()
        .flat_map(|entries| entries.handler_requests)
        .map(|request| request.name)
        .collect();
    let settings: Vec<&str> = modules
        .iter()
        .flat_map(|entries| entries.settings)
        .map(|setting| setting.name)
        .collect();
    let deprecations = modules.iter().flat_map(|entries| {
        let operations = entries
            .operations
            .iter()
            .map(|operation| (operation.name, operation.deprecated, &operations));
        let events = entries
            .events
            .iter()
            .map(|event| (event.name, event.deprecated, &events));
        let requests = entries
            .handler_requests
            .iter()
            .map(|request| (request.name, request.deprecated, &requests));
        let settings = entries
            .settings
            .iter()
            .map(|setting| (setting.name, setting.deprecated, &settings));
        operations.chain(events).chain(requests).chain(settings)
    });

    let wrong: Vec<&str> = deprecations
        .filter_map(|(name, deprecated, section)| {
            let deprecated = deprecated?;
            let replaced = deprecated
                .replacement
                .is_none_or(|replacement| replacement != name && section.contains(&replacement));
            let later = deprecated.earliest_removal.major > deprecated.since.major
                && deprecated.earliest_removal.minor == 0;
            (!(replaced && later)).then_some(name)
        })
        .collect();

    assert_eq!(wrong, Vec::<&str>::new());
}
