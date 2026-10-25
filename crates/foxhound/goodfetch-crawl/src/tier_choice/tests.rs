//! Covers: story:6-257

use goodfetch_fetch_tier::TierChoice;
use goodfetch_protocol::foxhound::crawl::PushedRecord;
use goodfetch_protocol::foxhound::fetch::FetchTier;
use rusqlite::Connection;
use serde_json::json;

use super::{
    AttemptPlan, MAX_SEGMENTS, SAMPLES, Sampled, TABLES, TierChoices, matches, read, record,
    segment,
};

const KEY: &str = "tiers.test";

/// The plan of a request of `label` at `path` on [`KEY`], under `auto`.
fn plan(choices: &TierChoices, label: &str, path: &str) -> AttemptPlan {
    choices.serve_attempt(label, KEY, &format!("http://tiers.test{path}"))
}

/// Classifies the sample of a request of `label` at `path`, as matched or
/// not.
fn sample(choices: &mut TierChoices, label: &str, path: &str, matched: bool) {
    let AttemptPlan::Sample(sampled) = plan(choices, label, path) else {
        panic!("{path} is no sample: {:?}", plan(choices, label, path));
    };
    choices.classify(&sampled, matched);
}

/// The tier a request at `path` takes once the choice is made, `None`
/// while it is still a sample.
fn tier(choices: &TierChoices, label: &str, path: &str) -> Option<FetchTier> {
    match plan(choices, label, path) {
        AttemptPlan::On(TierChoice { tier, forced: None }) => Some(tier),
        AttemptPlan::Sample(_) => None,
        other => panic!("{path}: {other:?}"),
    }
}

/// A Label whose every sample matched is learned onto `http`, one whose
/// every sample differed onto `browser`, each after its samples only, and
/// each Label on its own.
#[test]
fn a_label_is_learned_onto_http_when_every_sample_matches_and_onto_browser_otherwise() {
    let mut choices = TierChoices::default();
    let mut before = Vec::new();
    for page in 1..=SAMPLES {
        before.push((
            tier(&choices, "static", &format!("/products/{page}")),
            tier(&choices, "scripted", &format!("/blog/{page}")),
        ));
        sample(&mut choices, "static", &format!("/products/{page}"), true);
        sample(&mut choices, "scripted", &format!("/blog/{page}"), false);
    }

    assert_eq!(
        (
            before,
            tier(&choices, "static", "/products/9"),
            tier(&choices, "scripted", "/blog/9"),
            tier(&choices, "other", "/products/9"),
        ),
        (
            vec![(None, None); SAMPLES],
            Some(FetchTier::Http),
            Some(FetchTier::Browser),
            None,
        )
    );
}

/// Samples that disagree by first path segment split the Label: the
/// segment whose samples all matched goes on `http` once it has its own
/// three, the one whose samples differed stays on `browser`, and a
/// segment whose own samples disagree stays on `browser`.
///
/// Covers: story:6-258
#[test]
fn samples_that_disagree_by_first_segment_split_the_label_and_a_segment_that_disagrees_stays_on_browser()
 {
    let mut choices = TierChoices::default();
    sample(&mut choices, "page", "/products/1", true);
    sample(&mut choices, "page", "/blog/1", false);
    sample(&mut choices, "page", "/products/2", true);
    let split = (
        tier(&choices, "page", "/products/3"),
        tier(&choices, "page", "/blog/2"),
    );
    sample(&mut choices, "page", "/products/3", true);
    sample(&mut choices, "page", "/mixed/1", true);
    sample(&mut choices, "page", "/mixed/2", false);

    assert_eq!(
        (
            split,
            tier(&choices, "page", "/products/4"),
            tier(&choices, "page", "/blog/3"),
            tier(&choices, "page", "/mixed/3"),
            tier(&choices, "page", "/"),
        ),
        (
            (None, Some(FetchTier::Browser)),
            Some(FetchTier::Http),
            Some(FetchTier::Browser),
            Some(FetchTier::Browser),
            None,
        )
    );
}

/// A split Label keeps at most 16 segments: of 17 segments, the 17th takes
/// the Label's own choice, `browser`, and is never sampled.
///
/// Covers: story:6-258
#[test]
fn a_split_label_keeps_sixteen_segments_and_the_rest_share_the_labels_choice() {
    let mut choices = TierChoices::default();
    sample(&mut choices, "page", "/s1/1", true);
    sample(&mut choices, "page", "/s2/1", false);
    sample(&mut choices, "page", "/s1/2", true);
    for segment in 3..=MAX_SEGMENTS {
        sample(&mut choices, "page", &format!("/s{segment}/1"), true);
    }
    let kept: Vec<String> = (1..=MAX_SEGMENTS)
        .map(|segment| format!("s{segment}"))
        .collect();

    assert_eq!(
        (
            choices.segments("page", KEY),
            tier(&choices, "page", "/s3/2"),
            tier(&choices, "page", "/s17/1"),
            tier(&choices, "page", "/s18/1"),
        ),
        (
            kept,
            None,
            Some(FetchTier::Browser),
            Some(FetchTier::Browser),
        )
    );
}

/// What each Label on each domain learned reads back from its run file as
/// it was written, so a resumed crawl takes the same tiers.
///
/// Covers: story:6-257
#[test]
fn the_learned_choices_read_back_from_the_run_file() {
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
        connection.execute_batch(statement).unwrap();
    }
    let mut choices = TierChoices::default();
    for page in 1..=SAMPLES {
        sample(&mut choices, "static", &format!("/products/{page}"), true);
    }
    sample(&mut choices, "page", "/products/1", true);
    sample(&mut choices, "page", "/blog/1", false);
    sample(&mut choices, "page", "/products/2", true);
    for (label, key, unit) in choices.units() {
        record(&connection, label, key, unit).unwrap();
    }

    let read = read(&connection).unwrap();

    assert_eq!(
        (
            tier(&read, "static", "/products/9"),
            tier(&read, "page", "/blog/9"),
            tier(&read, "page", "/products/9"),
            &read,
        ),
        (
            Some(FetchTier::Http),
            Some(FetchTier::Browser),
            None,
            &choices,
        )
    );
}

/// The unit a request is learned under: its Label, its domain key and the
/// first segment of its URL's path.
#[test]
fn a_request_is_sampled_under_its_label_its_domain_key_and_its_first_segment() {
    let choices = TierChoices::default();
    assert_eq!(
        (
            plan(&choices, "page", "/products/1?page=2"),
            segment("http://tiers.test/"),
            segment("http://tiers.test"),
            segment("not a url"),
        ),
        (
            AttemptPlan::Sample(Sampled {
                label: "page".to_owned(),
                domain_key: KEY.to_owned(),
                segment: "products".to_owned(),
            }),
            String::new(),
            String::new(),
            String::new(),
        )
    );
}

fn pushed(data: serde_json::Value) -> PushedRecord {
    PushedRecord {
        dataset: "default".to_owned(),
        data,
    }
}

/// The stand-in weighing until #768's: the same Records in any order
/// match, a missing, extra or changed Record does not, and a cheaper run
/// that escalated or failed never matches.
#[test]
fn two_runs_match_when_they_pushed_the_same_records_in_any_order() {
    let one = pushed(json!({ "name": "Bench", "price": 3 }));
    let two = pushed(json!({ "price": 4, "name": "Clock" }));
    assert_eq!(
        [
            matches(
                Some(&[one.clone(), two.clone()]),
                &[two.clone(), one.clone()]
            ),
            matches(
                Some(std::slice::from_ref(&one)),
                &[one.clone(), two.clone()]
            ),
            matches(
                Some(&[one.clone(), one.clone()]),
                &[one.clone(), two.clone()]
            ),
            matches(
                Some(&[pushed(json!({ "name": "Bench" }))]),
                std::slice::from_ref(&one)
            ),
            matches(None, &[one]),
            matches(Some(&[]), &[]),
        ],
        [true, false, false, false, false, true]
    );
}
