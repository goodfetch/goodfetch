//! The resolver of a page's main-content step: each built-in from the
//! nearest scope that names one, the attempts from the Run alone.

use goodfetch_engine::settings::{Layers, Resolved, Values, resolve};
use goodfetch_extractor::{Attempts, BuiltIn, Chosen};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;
use serde_json::{Value, json};

use super::{chosen, served};

/// `given` read as a Run's own settings, or a Session's or a page's.
fn values(given: Value) -> Values {
    let Value::Object(given) = given else {
        unreachable!("a JSON object");
    };
    Values::read(SettingsLayer::Run, &given, Product::Collie).unwrap()
}

/// A Run's settings resolved from `given`, its own layer.
fn run(given: Value) -> Resolved {
    let layers = Layers {
        run: values(given),
        ..Layers::default()
    };
    resolve(&layers, Product::Collie, Product::Collie).unwrap()
}

fn extractor(name: &str) -> Value {
    json!({ "content": { "extractor": name } })
}

/// Covers: setting:content.extractor, setting:extraction.readability
#[test]
fn with_no_scope_naming_either_the_step_is_dom_smoothie_with_retries() {
    let defaults = run(json!({}));

    assert_eq!(
        (
            chosen(None, (&Values::default(), None), &Values::default()),
            chosen(
                Some(&defaults),
                (&Values::default(), None),
                &Values::default()
            ),
        ),
        (Chosen::default(), Chosen::default())
    );
}

/// The page's own settings win over its Label's, which win over its
/// Session's, which win over its Run's.
///
/// Covers: setting:content.extractor
#[rstest]
#[case::the_page(
    Some("trafilatura"),
    Some(BuiltIn::DomSmoothie),
    Some("dom_smoothie"),
    "dom_smoothie",
    BuiltIn::Trafilatura
)]
#[case::the_label(
    None,
    Some(BuiltIn::Trafilatura),
    Some("dom_smoothie"),
    "dom_smoothie",
    BuiltIn::Trafilatura
)]
#[case::the_session(None, None, Some("trafilatura"), "dom_smoothie", BuiltIn::Trafilatura)]
#[case::the_run(None, None, None, "trafilatura", BuiltIn::Trafilatura)]
fn the_nearest_scope_naming_a_built_in_chooses_it(
    #[case] page: Option<&str>,
    #[case] label: Option<BuiltIn>,
    #[case] session: Option<&str>,
    #[case] by_run: &str,
    #[case] expected: BuiltIn,
) {
    let scoped =
        |name: Option<&str>| name.map_or_else(Values::default, |name| values(extractor(name)));
    let by_run = run(extractor(by_run));

    let chosen = chosen(Some(&by_run), (&scoped(session), label), &scoped(page));

    assert_eq!(chosen.built_in, expected);
}

/// `extraction.readability` is the Run's, whichever scope chose the
/// built-in.
///
/// Covers: setting:extraction.readability
#[rstest]
fn the_attempts_are_the_runs_whichever_scope_chose_the_built_in(
    #[values("retry", "single_attempt")] attempts: &str,
    #[values(None, Some(BuiltIn::Trafilatura))] label: Option<BuiltIn>,
) {
    let by_run = run(json!({ "extraction": { "readability": attempts } }));
    let session = values(extractor("trafilatura"));

    let chosen = chosen(Some(&by_run), (&session, label), &Values::default());

    assert_eq!(
        (chosen.built_in, Some(chosen.attempts)),
        (BuiltIn::Trafilatura, Attempts::named(attempts))
    );
}

/// A served document's step, the Page signature's and its formats', is
/// what a page with no Session's or page's settings resolves.
///
/// Covers: setting:content.extractor, setting:extraction.readability
#[rstest]
fn a_served_documents_step_is_the_one_a_page_with_no_settings_of_its_own_resolves(
    #[values(None, Some(BuiltIn::Trafilatura))] label: Option<BuiltIn>,
    #[values(
        json!({}),
        json!({ "content": { "extractor": "trafilatura" } }),
        json!({ "extraction": { "readability": "single_attempt" } })
    )]
    given: Value,
) {
    let by_run = run(given);

    assert_eq!(
        served(Some(&by_run), label),
        chosen(
            Some(&by_run),
            (&Values::default(), label),
            &Values::default()
        )
    );
}
