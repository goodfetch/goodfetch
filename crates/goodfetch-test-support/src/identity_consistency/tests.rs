use rstest::rstest;
use serde_json::{Value, json};

use super::{as_headers_send, header_value};
use crate::proving_ground_format::ObservedRequest;

/// A report request carrying the header `name` as `value`.
fn request_with(name: &str, value: &str) -> ObservedRequest {
    serde_json::from_value(json!({
        "sequence": 0,
        "connection": 0,
        "task": null,
        "host": "consistency.test",
        "protocol": "h2",
        "kind": "request",
        "method": "POST",
        "target": "/reports?page=/",
        "pseudo_header_order": null,
        "headers": [[name, value]],
        "cookies": [],
        "message_signature": null,
        "tls": null,
        "http2": null,
        "status": 204,
        "timing": { "arrived_microseconds": 0, "answered_microseconds": 1 },
        "bytes": {},
        "body": {},
    }))
    .unwrap()
}

/// Each header is read into the spelling the probes report the surface in,
/// so a header and a script that agree compare equal.
#[rstest]
#[case::brand_list(
    "ua_brands",
    "sec-ch-ua",
    r#""Not:A-Brand";v="8", "Chromium";v="156""#,
    json!(["Not:A-Brand/8", "Chromium/156"])
)]
#[case::escaped_string(
    "ua_model",
    "sec-ch-ua-model",
    r#""Pixel \"9\" \\ Pro""#,
    json!(r#"Pixel "9" \ Pro"#)
)]
#[case::empty_string("ua_platform_version", "sec-ch-ua-platform-version", r#""""#, json!(""))]
#[case::boolean("ua_mobile", "Sec-CH-UA-Mobile", "?1", json!(true))]
#[case::first_language("language", "accept-language", "fr-FR, fr;q=0.9, en;q=0.8", json!("fr-FR"))]
#[case::languages(
    "languages",
    "accept-language",
    "fr-FR, fr;q=0.9, en;q=0.8",
    json!(["fr-FR", "fr", "en"])
)]
#[case::text("user_agent", "User-Agent", "Mozilla/5.0 (X11)", json!("Mozilla/5.0 (X11)"))]
fn a_header_is_read_as_the_probes_spell_its_surface(
    #[case] surface: &str,
    #[case] name: &str,
    #[case] sent: &str,
    #[case] expected: Value,
) {
    let request = request_with(name, sent);

    let value = header_value(surface, &request);

    assert_eq!(value, Some(expected));
}

/// A header that is not what its surface's spelling describes is kept as
/// sent, so it differs from every value a script reports and is flagged,
/// never dropped.
#[rstest]
#[case::brand_without_version("ua_brands", "sec-ch-ua", r#""Chromium", "Brand";v="1""#)]
#[case::unclosed_string("ua_platform", "sec-ch-ua-platform", r#""Linux"#)]
#[case::trailing_bytes("ua_platform", "sec-ch-ua-platform", r#""Linux" x"#)]
#[case::token_boolean("ua_wow64", "sec-ch-ua-wow64", "false")]
fn a_header_its_spelling_cannot_read_is_kept_as_sent(
    #[case] surface: &str,
    #[case] name: &str,
    #[case] sent: &str,
) {
    let request = request_with(name, sent);

    let value = header_value(surface, &request);

    assert_eq!(value, Some(json!({ "header": name, "unread": sent })));
}

/// No header carries a surface such as the screen's, and a request that
/// sent no header of a surface gives nothing to compare.
#[rstest]
#[case::no_header_carries_it("screen_width")]
#[case::not_sent("ua_bitness")]
fn a_surface_no_header_sent_gives_nothing(#[case] surface: &str) {
    let request = request_with("user-agent", "Mozilla/5.0");

    let value = header_value(surface, &request);

    assert_eq!(value, None);
}

/// A language list is compared with its headers as Chromium expands it
/// into `Accept-Language`; every other surface as it is.
#[rstest]
#[case::base_added("languages", json!(["en-US"]), json!(["en-US", "en"]))]
#[case::base_after_its_family(
    "languages",
    json!(["en-US", "en-GB", "fr"]),
    json!(["en-US", "en-GB", "en", "fr"])
)]
#[case::base_already_listed("languages", json!(["fr-FR", "fr", "en"]), json!(["fr-FR", "fr", "en"]))]
#[case::base_never_twice(
    "languages",
    json!(["en-US", "fr", "en"]),
    json!(["en-US", "en", "fr"])
)]
#[case::private_base_not_added("languages", json!(["x-klingon", "de"]), json!(["x-klingon", "de"]))]
#[case::first_language_as_is("language", json!("en-US"), json!("en-US"))]
fn a_main_frame_value_is_compared_as_the_headers_send_it(
    #[case] surface: &str,
    #[case] reported: Value,
    #[case] sent: Value,
) {
    assert_eq!(as_headers_send(surface, &reported), sent);
}
