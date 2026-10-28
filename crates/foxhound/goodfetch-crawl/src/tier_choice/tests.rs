//! Covers: story:6-257

use goodfetch_fetch_tier::TierChoice;
use goodfetch_protocol::foxhound::fetch::FetchTier;
use rusqlite::Connection;

use super::{
    AttemptPlan, MAX_SEGMENTS, Sampled, Sampling, TABLES, TierChoices, Weighed, read, record,
    segment,
};

const KEY: &str = "tiers.test";

/// The built-in `fetch.sample.count`.
const SAMPLES: usize = 3;

/// The moment every test starts at, in milliseconds since the Unix epoch.
const START: u64 = 1_790_000_000_000;

/// The plan of a request of `label` at `path` on [`KEY`], under `auto`, at
/// [`START`].
fn plan(choices: &mut TierChoices, label: &str, path: &str) -> AttemptPlan {
    plan_at(choices, label, path, START)
}

/// The plan of a request of `label` at `path` on [`KEY`] at `now`.
fn plan_at(choices: &mut TierChoices, label: &str, path: &str, now: u64) -> AttemptPlan {
    choices
        .serve_attempt(label, KEY, &format!("http://tiers.test{path}"), now)
        .0
}

/// Classifies the sample of a request of `label` at `path`, as matched or
/// not.
fn sample(choices: &mut TierChoices, label: &str, path: &str, matched: bool) {
    let weighed = if matched {
        Weighed::Matched
    } else {
        Weighed::Differed
    };
    weigh(choices, label, path, weighed);
}

/// Classifies the sample of a request of `label` at `path` as `weighed`.
fn weigh(choices: &mut TierChoices, label: &str, path: &str, weighed: Weighed) {
    let AttemptPlan::Sample(sampled) = plan(choices, label, path) else {
        panic!("{path} is no sample");
    };
    choices.classify(&sampled, weighed, START);
}

/// The tier a request at `path` takes once the choice is made, `None`
/// while it is still a sample.
fn tier(choices: &mut TierChoices, label: &str, path: &str) -> Option<FetchTier> {
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
            tier(&mut choices, "static", &format!("/products/{page}")),
            tier(&mut choices, "scripted", &format!("/blog/{page}")),
        ));
        sample(&mut choices, "static", &format!("/products/{page}"), true);
        sample(&mut choices, "scripted", &format!("/blog/{page}"), false);
    }

    assert_eq!(
        (
            before,
            tier(&mut choices, "static", "/products/9"),
            tier(&mut choices, "scripted", "/blog/9"),
            tier(&mut choices, "other", "/products/9"),
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
        tier(&mut choices, "page", "/products/3"),
        tier(&mut choices, "page", "/blog/2"),
    );
    sample(&mut choices, "page", "/products/3", true);
    sample(&mut choices, "page", "/mixed/1", true);
    sample(&mut choices, "page", "/mixed/2", false);

    assert_eq!(
        (
            split,
            tier(&mut choices, "page", "/products/4"),
            tier(&mut choices, "page", "/blog/3"),
            tier(&mut choices, "page", "/mixed/3"),
            tier(&mut choices, "page", "/"),
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
            tier(&mut choices, "page", "/s3/2"),
            tier(&mut choices, "page", "/s17/1"),
            tier(&mut choices, "page", "/s18/1"),
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

    let mut read = read(&connection).unwrap();
    let same = read == choices;

    assert_eq!(
        (
            same,
            tier(&mut read, "static", "/products/9"),
            tier(&mut read, "page", "/blog/9"),
            tier(&mut read, "page", "/products/9"),
        ),
        (true, Some(FetchTier::Http), Some(FetchTier::Browser), None,)
    );
}

/// The unit a request is learned under: its Label, its domain key and the
/// first segment of its URL's path.
#[test]
fn a_request_is_sampled_under_its_label_its_domain_key_and_its_first_segment() {
    let mut choices = TierChoices::default();
    assert_eq!(
        (
            plan(&mut choices, "page", "/products/1?page=2"),
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

/// A choice is sampled again once it served `fetch.sample.refresh_after`'s
/// requests, or once its time passed, whichever comes first: its next
/// request is a sample again, neither a retry nor a failure, and a choice
/// the samples then make stands anew.
///
/// Covers: story:5-114
#[test]
fn a_choice_is_sampled_again_after_its_requests_or_its_time_whichever_comes_first() {
    let mut choices = TierChoices::default().sampled_as(Sampling {
        count: SAMPLES,
        requests: 2,
        after_ms: 60_000,
    });
    for page in 1..=SAMPLES {
        sample(&mut choices, "by-count", &format!("/products/{page}"), true);
        sample(&mut choices, "by-time", &format!("/products/{page}"), true);
    }
    let by_count = [
        tier(&mut choices, "by-count", "/products/4"),
        tier(&mut choices, "by-count", "/products/5"),
        tier(&mut choices, "by-count", "/products/6"),
    ];
    let by_time = [
        plan_at(&mut choices, "by-time", "/products/4", START + 59_999),
        plan_at(&mut choices, "by-time", "/products/5", START + 60_000),
    ];

    assert_eq!(
        (by_count, by_time),
        (
            [Some(FetchTier::Http), Some(FetchTier::Http), None],
            [
                AttemptPlan::On(TierChoice {
                    tier: FetchTier::Http,
                    forced: None
                }),
                AttemptPlan::Sample(Sampled {
                    label: "by-time".to_owned(),
                    domain_key: KEY.to_owned(),
                    segment: "products".to_owned(),
                }),
            ]
        )
    );
}

/// A resumed crawl goes on counting the requests a choice served: the count
/// is kept in the run file beside the moment the choice was made, so a
/// crawl resumed more often than `fetch.sample.refresh_after`'s requests is
/// still sampled again after them.
///
/// Covers: story:5-114, story:6-257
#[test]
fn a_resumed_crawl_goes_on_counting_the_requests_a_choice_served() {
    let sampling = Sampling {
        count: SAMPLES,
        requests: 2,
        after_ms: 60_000,
    };
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
        connection.execute_batch(statement).unwrap();
    }
    let mut choices = TierChoices::default().sampled_as(sampling);
    for page in 1..=SAMPLES {
        sample(&mut choices, "static", &format!("/products/{page}"), true);
    }
    let before = tier(&mut choices, "static", "/products/4");
    for (label, key, unit) in choices.units() {
        record(&connection, label, key, unit).unwrap();
    }

    let mut resumed = read(&connection).unwrap().sampled_as(sampling);
    let after = [
        tier(&mut resumed, "static", "/products/5"),
        tier(&mut resumed, "static", "/products/6"),
    ];

    assert_eq!(
        (before, after),
        (Some(FetchTier::Http), [Some(FetchTier::Http), None])
    );
}

/// A declared total that stops matching samples its Label on its domain
/// again at once, whatever it served (ADR 0020); other Labels keep theirs.
///
/// Covers: story:5-114
#[test]
fn a_declared_total_that_stops_matching_samples_the_label_again_at_once() {
    let mut choices = TierChoices::default();
    for page in 1..=SAMPLES {
        sample(&mut choices, "listing", &format!("/products/{page}"), true);
        sample(&mut choices, "other", &format!("/products/{page}"), true);
    }
    let before = tier(&mut choices, "listing", "/products/9");
    choices.invalidate("listing", KEY);

    assert_eq!(
        (
            before,
            tier(&mut choices, "listing", "/products/9"),
            tier(&mut choices, "other", "/products/9"),
        ),
        (Some(FetchTier::Http), None, Some(FetchTier::Http))
    );
}

/// A Label whose browser sample showed a consent wall stays on `browser`
/// for every request and segment, though its other samples matched, and
/// is never sampled again, by its count, its time or a declared total
/// (ADR 0048).
///
/// Covers: story:5-117
#[test]
fn a_label_whose_browser_sample_showed_a_consent_wall_never_moves_to_http() {
    let mut choices = TierChoices::default().sampled_as(Sampling {
        count: SAMPLES,
        requests: 1,
        after_ms: 1,
    });
    sample(&mut choices, "page", "/products/1", true);
    weigh(&mut choices, "page", "/products/2", Weighed::Walled);
    choices.invalidate("page", KEY);

    assert_eq!(
        (
            tier(&mut choices, "page", "/products/3"),
            plan_at(&mut choices, "page", "/blog/1", START + 1_000_000),
        ),
        (
            Some(FetchTier::Browser),
            AttemptPlan::On(TierChoice {
                tier: FetchTier::Browser,
                forced: None
            })
        )
    );
}

/// `fetch.sample.count` sets how many samples a Label takes: with one, its
/// first matching sample moves it to `http`.
///
/// Covers: story:5-114
#[test]
fn fetch_sample_count_sets_how_many_samples_a_label_takes() {
    let mut choices = TierChoices::default().sampled_as(Sampling {
        count: 1,
        ..Sampling::default()
    });
    sample(&mut choices, "page", "/products/1", true);

    assert_eq!(
        tier(&mut choices, "page", "/products/2"),
        Some(FetchTier::Http)
    );
}

/// Where the Run allows the light-browser tier, a Label whose samples
/// all matched, some only on `light_browser`, is learned onto
/// `light_browser`; one whose every sample matched over `http` still goes
/// on `http`, the cheaper; and one with a sample that matched on neither
/// stays on `browser` (ADR 0020). A split Label's segment follows the same
/// rule.
///
/// Covers: story:5-120
#[test]
fn a_label_whose_samples_matched_only_with_the_light_browser_is_learned_onto_it() {
    let mut choices = TierChoices::default();
    for page in 1..=SAMPLES {
        let light = if page == 2 {
            Weighed::Matched
        } else {
            Weighed::MatchedLight
        };
        weigh(&mut choices, "scripted", &format!("/blog/{page}"), light);
        weigh(
            &mut choices,
            "static",
            &format!("/products/{page}"),
            Weighed::Matched,
        );
        let painted = if page == 3 {
            Weighed::Differed
        } else {
            Weighed::MatchedLight
        };
        weigh(
            &mut choices,
            "painted",
            &format!("/painted/{page}"),
            painted,
        );
    }
    let mut split = TierChoices::default();
    weigh(&mut split, "mixed", "/blog/1", Weighed::MatchedLight);
    weigh(&mut split, "mixed", "/painted/1", Weighed::Differed);
    for page in 2..=SAMPLES {
        weigh(
            &mut split,
            "mixed",
            &format!("/blog/{page}"),
            Weighed::MatchedLight,
        );
    }

    assert_eq!(
        (
            tier(&mut choices, "scripted", "/blog/9"),
            tier(&mut choices, "static", "/products/9"),
            tier(&mut choices, "painted", "/painted/9"),
            tier(&mut split, "mixed", "/blog/9"),
            tier(&mut split, "mixed", "/painted/9"),
        ),
        (
            Some(FetchTier::LightBrowser),
            Some(FetchTier::Http),
            Some(FetchTier::Browser),
            Some(FetchTier::LightBrowser),
            Some(FetchTier::Browser),
        )
    );
}

/// The plan a `discovered_api` sample run replayed on `/scroll/<n>`: a
/// page-number listing.
fn replay_plan(n: u32) -> goodfetch_fetch_tier::ReplayPlan {
    let document =
        goodfetch_frontier::Url::parse(&format!("http://tiers.test/scroll/{n}")).unwrap();
    let calls: Vec<goodfetch_fetch_tier::CapturedCall> = (1..=2)
        .map(|page| goodfetch_fetch_tier::CapturedCall {
            url: goodfetch_frontier::Url::parse(&format!(
                "http://tiers.test/api/scroll/{n}?page={page}"
            ))
            .unwrap(),
            json: serde_json::json!({ "items": [page] }),
        })
        .collect();
    goodfetch_fetch_tier::infer_plan(&document, &calls).unwrap()
}

/// A Label whose samples all matched, those that differed over `http`
/// matching on `discovered_api`, is learned onto `discovered_api`, its
/// later requests replaying the last such sample's plan; one with a
/// sample that matched only on `light_browser` and others only on
/// `discovered_api` goes on `browser`, neither tier having served every
/// one (ADR 0020).
///
/// Covers: story:5-119
#[test]
fn a_label_whose_samples_matched_on_discovered_api_is_learned_onto_it_with_its_plan() {
    let mut choices = TierChoices::default();
    for page in 1..=SAMPLES {
        let number = u32::try_from(page).unwrap();
        let weighed = if page == 2 {
            Weighed::Matched
        } else {
            Weighed::MatchedApi {
                plan: replay_plan(number),
                light: false,
            }
        };
        weigh(&mut choices, "scroll", &format!("/scroll/{page}"), weighed);
        let mixed = if page == 1 {
            Weighed::MatchedLight
        } else {
            Weighed::MatchedApi {
                plan: replay_plan(number),
                light: false,
            }
        };
        weigh(&mut choices, "mixed", &format!("/mixed/{page}"), mixed);
    }

    assert_eq!(
        plan(&mut choices, "scroll", "/scroll/9"),
        AttemptPlan::Replayed(replay_plan(3))
    );
    assert_eq!(
        tier(&mut choices, "mixed", "/mixed/9"),
        Some(FetchTier::Browser)
    );
}
