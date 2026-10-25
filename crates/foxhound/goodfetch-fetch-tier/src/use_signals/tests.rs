//! The AI-use signals' judgment as a function of a response's headers
//! and the Purpose (spec #581's pure rules, tested in process).

use rstest::rstest;

use super::{Purpose, UseSignals};

fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

/// Which of `ai-input`, `ai-train` and `collect` the headers refuse.
fn refused(pairs: &[(&str, &str)]) -> [bool; 3] {
    let signals = UseSignals::of(&headers(pairs));
    [Purpose::AiInput, Purpose::AiTrain, Purpose::Collect]
        .map(|purpose| signals.refusing(purpose).is_some())
}

#[rstest]
#[case::no_signal(&[], [false, false, false])]
#[case::a_tdm_reservation(&[("tdm-reservation", "1")], [true, true, true])]
#[case::a_tdm_reservation_in_any_case(&[("TDM-Reservation", " 1 ")], [true, true, true])]
#[case::no_tdm_reservation(&[("tdm-reservation", "0")], [false, false, false])]
#[case::aipref_refusing_training(&[("content-usage", "train-ai=n")], [false, true, false])]
#[case::aipref_refusing_ai_use(&[("content-usage", "ai-use=n, search=y")], [true, false, false])]
#[case::aipref_refusing_all_processing(&[("content-usage", "bulk=n")], [true, true, true])]
#[case::the_narrower_preference_wins(&[("content-usage", "bulk=n, train-ai=y")], [true, false, true])]
#[case::aipref_allowing(&[("content-usage", "train-ai=y")], [false, false, false])]
#[case::aipref_unknown_values_say_nothing(&[("content-usage", "train-ai, ai-use=maybe")], [false, false, false])]
fn a_signal_refuses_the_purposes_it_names(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: [bool; 3],
) {
    assert_eq!(refused(pairs), expected);
}

#[test]
fn no_signal_is_judged_against_search() {
    let signals = UseSignals::of(&headers(&[
        ("tdm-reservation", "1"),
        ("content-usage", "bulk=n, search=n"),
    ]));

    assert_eq!(signals.refusing(Purpose::Search), None);
}

#[rstest]
#[case("search", Some(Purpose::Search))]
#[case("ai-input", Some(Purpose::AiInput))]
#[case("ai-train", Some(Purpose::AiTrain))]
#[case("collect", Some(Purpose::Collect))]
#[case("training", None)]
fn a_purpose_is_named_as_the_setting_names_it(
    #[case] name: &str,
    #[case] expected: Option<Purpose>,
) {
    assert_eq!(Purpose::named(name), expected);
}
