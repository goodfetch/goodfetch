//! The font pages (#328, ADR 0074) on `fonts.test`: a known text in each
//! font the Identity profile lists, in the fallback, and in each host font
//! outside the profile, each with the result a browser presenting that
//! profile shows. A plain HTTP client reads the page; a stock Chromium,
//! never Goodfetch, renders it and shows what it finds.

use serde_json::{Value, json};

use crate::browser;
use crate::proving_ground::ProvingGround;

const SEED: u64 = 328;

const HOST: &str = "fonts.test";

/// A family no host has.
const ABSENT: &str = "Goodfetch Absent Font";

/// The page for a profile that lists `families`.
fn page_url(proving_ground: &ProvingGround, families: &[&str]) -> String {
    let query: Vec<String> = families
        .iter()
        .map(|family| format!("family={}", family.replace(' ', "+")))
        .collect();
    proving_ground.url(HOST, &format!("/?{}", query.join("&")))
}

/// What a stock Chromium shows of the page for `families`: each font's
/// `listed`, `expected` and `observed`, and the fallback's text size.
fn rendered(proving_ground: &ProvingGround, families: &[&str]) -> Value {
    browser::run(
        "fonts",
        "fonts",
        &page_url(proving_ground, families),
        json!({}),
    )
}

/// The families of `fonts` whose field `field` is `value`.
fn families_where(fonts: &Value, field: &str, value: &Value) -> Vec<String> {
    fonts
        .as_object()
        .expect("the fonts by family")
        .iter()
        .filter(|(_, font)| &font[field] == value)
        .map(|(family, _)| family.clone())
        .collect()
}

/// Covers: story:2-91
#[test]
fn the_page_lists_the_profiles_fonts_and_every_host_font_outside_it_with_its_expected_result() {
    let proving_ground = ProvingGround::start(SEED);

    let page = proving_ground.get(&page_url(&proving_ground, &["Arial", ABSENT]));
    let script = proving_ground.get_raw(&proving_ground.url(HOST, "/fonts.js"));
    let unsafe_name = proving_ground.get(&proving_ground.url(HOST, "/?family=a%22b"));

    assert_eq!(page.status, 200);
    for listed in ["Arial", ABSENT] {
        assert!(
            page.body.contains(&format!(
                "data-family=\"{listed}\" data-listed=\"true\" data-expected=\"available\""
            )),
            "{listed} is expected available: {}",
            page.body
        );
    }
    for host_font in ["DejaVu Sans", "Segoe UI", "Helvetica Neue"] {
        assert!(
            page.body.contains(&format!(
                "data-family=\"{host_font}\" data-listed=\"false\" data-expected=\"unavailable\""
            )),
            "{host_font} is probed and expected unavailable: {}",
            page.body
        );
    }
    assert_eq!(
        page.body.matches("data-family=\"Arial\"").count(),
        1,
        "a listed font is not probed again"
    );
    assert!(page.body.contains("id=\"fallback\""), "{}", page.body);
    assert_eq!(
        (script.status, script.content_type.as_deref()),
        (200, Some("application/javascript"))
    );
    assert_eq!(unsafe_name.status, 400, "{}", unsafe_name.body);
}

/// Covers: story:2-91
#[test]
fn a_stock_chromium_shows_which_fonts_render_apart_from_the_fallback() {
    let proving_ground = ProvingGround::start(SEED);

    let unlisted = rendered(&proving_ground, &[ABSENT]);
    let host_fonts = families_where(&unlisted["fonts"], "observed", &json!("available"));
    let listed: Vec<&str> = host_fonts.iter().map(String::as_str).collect();
    let profile = rendered(&proving_ground, &listed);

    assert!(
        unlisted["fallback"]["width"]
            .as_f64()
            .is_some_and(|width| width > 0.0),
        "the fallback renders the text: {unlisted}"
    );
    assert_eq!(
        unlisted["fonts"][ABSENT],
        json!({ "listed": true, "expected": "available", "observed": "unavailable" }),
        "a listed font the browser lacks is found missing"
    );
    let mismatched: Vec<_> = profile["fonts"]
        .as_object()
        .expect("the fonts by family")
        .iter()
        .filter(|(_, font)| font["observed"] != font["expected"])
        .collect();
    assert!(
        mismatched.is_empty(),
        "a profile of the host's own fonts matches the stock browser: {mismatched:?}"
    );
}
