//! Covers: story:6-257

#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_fetch_tier::{Forced, HttpFrom, TierChoice};
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_session_pool::SendsFrom;
use rusqlite::Connection;

use super::{
    AttemptPlan, Between, MAX_SEGMENTS, Replay, Sampled, Sampling, TABLES, TierChoices, Unit,
    Weighed, encoded, read, record, segment,
};

const KEY: &str = "tiers.test";

/// The built-in `fetch.sample.count`.
const SAMPLES: usize = 3;

/// The moment every test starts at, in milliseconds since the Unix epoch.
const START: u64 = 1_790_000_000_000;

/// The segments `label` keeps a tier for on `domain_key`.
fn segments(choices: &TierChoices, label: &str, domain_key: &str) -> Vec<String> {
    match choices
        .units
        .get(&(label.to_owned(), domain_key.to_owned()))
    {
        Some(Unit::Split { segments, .. }) => segments
            .iter()
            .map(|segment| segment.name.clone())
            .collect(),
        _ => Vec::new(),
    }
}

/// The plan of a request of `label` at `path` on [`KEY`], under `auto`, at
/// [`START`].
fn plan(choices: &mut TierChoices, label: &str, path: &str) -> AttemptPlan {
    plan_at(choices, label, path, START)
}

/// The plan of a request of `label` at `path` on [`KEY`] at `now`, its
/// HTTP sent from the browser.
fn plan_at(choices: &mut TierChoices, label: &str, path: &str, now: u64) -> AttemptPlan {
    plan_from(choices, label, path, HttpFrom::Browser, now)
}

/// The plan of a request of `label` at `path` on [`KEY`] at `now`, its
/// HTTP sent from `from`.
fn plan_from(
    choices: &mut TierChoices,
    label: &str,
    path: &str,
    from: HttpFrom,
    now: u64,
) -> AttemptPlan {
    choices
        .serve_attempt(label, KEY, &format!("http://tiers.test{path}"), from, now)
        .0
}

/// A sample whose `http` run differed and no other run matched.
fn differed() -> Weighed {
    Weighed::Differed(Between::default())
}

/// A sample whose `http` run differed and whose `light_browser` run
/// matched.
fn light() -> Weighed {
    Weighed::Differed(Between {
        light: true,
        ..Between::default()
    })
}

/// A sample whose `http` run differed and whose `browser_fetch` run
/// matched.
fn fetched() -> Weighed {
    Weighed::Differed(Between {
        fetch: true,
        ..Between::default()
    })
}

/// Classifies the sample of a request of `label` at `path`, as matched or
/// not.
fn sample(choices: &mut TierChoices, label: &str, path: &str, matched: bool) {
    let weighed = if matched {
        Weighed::Matched
    } else {
        differed()
    };
    weigh(choices, label, path, weighed);
}

/// Classifies the sample of a request of `label` at `path` as `weighed`.
fn weigh(choices: &mut TierChoices, label: &str, path: &str, weighed: Weighed) {
    weigh_from(choices, label, path, HttpFrom::Browser, weighed);
}

/// Classifies the sample of a request of `label` at `path`, its HTTP sent
/// from `from`, as `weighed`.
fn weigh_from(
    choices: &mut TierChoices,
    label: &str,
    path: &str,
    from: HttpFrom,
    weighed: Weighed,
) {
    let AttemptPlan::Sample(sampled) = plan_from(choices, label, path, from, START) else {
        panic!("{path} is no sample");
    };
    choices.classify(&sampled, weighed, START);
}

/// The tier a request at `path` takes once the choice is made, `None`
/// while it is still a sample.
fn tier(choices: &mut TierChoices, label: &str, path: &str) -> Option<FetchTier> {
    tier_from(choices, label, path, HttpFrom::Browser)
}

/// The tier a request at `path`, its HTTP sent from `from`, takes once
/// the choice is made, `None` while it is still a sample.
fn tier_from(
    choices: &mut TierChoices,
    label: &str,
    path: &str,
    from: HttpFrom,
) -> Option<FetchTier> {
    match plan_from(choices, label, path, from, START) {
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
            segments(&choices, "page", KEY),
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
    for ((label, key), unit) in &choices.units {
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
                from: SendsFrom::Browser,
                browser_fetch: false,
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
                    from: SendsFrom::Browser,
                    browser_fetch: false,
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
    for ((label, key), unit) in &choices.units {
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
        let scripted = if page == 2 { Weighed::Matched } else { light() };
        weigh(&mut choices, "scripted", &format!("/blog/{page}"), scripted);
        weigh(
            &mut choices,
            "static",
            &format!("/products/{page}"),
            Weighed::Matched,
        );
        let painted = if page == 3 { differed() } else { light() };
        weigh(
            &mut choices,
            "painted",
            &format!("/painted/{page}"),
            painted,
        );
    }
    let mut split = TierChoices::default();
    weigh(&mut split, "mixed", "/blog/1", light());
    weigh(&mut split, "mixed", "/painted/1", differed());
    for page in 2..=SAMPLES {
        weigh(&mut split, "mixed", &format!("/blog/{page}"), light());
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

/// [`replay_plan`]`(n)`'s calls sent `from` that client.
fn replay(n: u32, from: SendsFrom) -> Replay {
    Replay {
        plan: replay_plan(n),
        from,
    }
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
            Weighed::Differed(Between {
                api: Some(replay(number, SendsFrom::Browser)),
                ..Between::default()
            })
        };
        weigh(&mut choices, "scroll", &format!("/scroll/{page}"), weighed);
        let mixed = if page == 1 {
            light()
        } else {
            Weighed::Differed(Between {
                api: Some(replay(number, SendsFrom::Browser)),
                ..Between::default()
            })
        };
        weigh(&mut choices, "mixed", &format!("/mixed/{page}"), mixed);
    }

    assert_eq!(
        plan(&mut choices, "scroll", "/scroll/9"),
        AttemptPlan::Replayed(replay(3, SendsFrom::Browser))
    );
    assert_eq!(
        tier(&mut choices, "mixed", "/mixed/9"),
        Some(FetchTier::Browser)
    );
}

/// Under `fetch.http_from: learned`, a Label whose samples that differed
/// over `http` all matched on `browser_fetch` is learned onto
/// `browser_fetch`; one with a sample whose browser-fetch run differed too
/// goes on `browser`. `browser_fetch` is dearer than `http` and
/// `discovered_api`, and cheaper than `light_browser`. A Label split by
/// segment keeps `browser_fetch` for the segment that needs it, `http` for
/// one that matched over `http` and `browser` for one that matched on
/// neither (#2417).
///
/// Covers: setting:fetch.http_from
#[test]
fn a_label_whose_samples_matched_only_on_browser_fetch_is_learned_onto_it() {
    let mut choices = TierChoices::default();
    let fetched_and_light = Weighed::Differed(Between {
        fetch: true,
        light: true,
        ..Between::default()
    });
    for page in 1..=SAMPLES {
        let number = u32::try_from(page).unwrap();
        let identity = if page == 2 {
            Weighed::Matched
        } else {
            fetched()
        };
        let scripted = if page == 3 { differed() } else { fetched() };
        let replayed = Weighed::Differed(Between {
            api: Some(replay(number, SendsFrom::Transport)),
            fetch: true,
            light: false,
        });
        for (label, path, weighed) in [
            ("identity", format!("/identity/{page}"), identity),
            ("scripted", format!("/scripted/{page}"), scripted),
            (
                "lighter",
                format!("/lighter/{page}"),
                fetched_and_light.clone(),
            ),
            ("scroll", format!("/scroll/{page}"), replayed),
        ] {
            weigh_from(&mut choices, label, &path, HttpFrom::Learned, weighed);
        }
    }
    let mut split = TierChoices::default();
    weigh_from(
        &mut split,
        "mixed",
        "/identity/1",
        HttpFrom::Learned,
        fetched(),
    );
    weigh_from(
        &mut split,
        "mixed",
        "/scripted/1",
        HttpFrom::Learned,
        differed(),
    );
    weigh_from(
        &mut split,
        "mixed",
        "/static/1",
        HttpFrom::Learned,
        Weighed::Matched,
    );
    for page in 2..=SAMPLES {
        weigh_from(
            &mut split,
            "mixed",
            &format!("/identity/{page}"),
            HttpFrom::Learned,
            fetched(),
        );
        weigh_from(
            &mut split,
            "mixed",
            &format!("/static/{page}"),
            HttpFrom::Learned,
            Weighed::Matched,
        );
    }

    assert_eq!(
        (
            tier_from(&mut choices, "identity", "/identity/9", HttpFrom::Learned),
            tier_from(&mut choices, "scripted", "/scripted/9", HttpFrom::Learned),
            tier_from(&mut choices, "lighter", "/lighter/9", HttpFrom::Learned),
            plan_from(
                &mut choices,
                "scroll",
                "/scroll/9",
                HttpFrom::Learned,
                START
            ),
            tier_from(&mut split, "mixed", "/identity/9", HttpFrom::Learned),
            tier_from(&mut split, "mixed", "/static/9", HttpFrom::Learned),
            tier_from(&mut split, "mixed", "/scripted/9", HttpFrom::Learned),
        ),
        (
            Some(FetchTier::BrowserFetch),
            Some(FetchTier::Browser),
            Some(FetchTier::BrowserFetch),
            AttemptPlan::Replayed(replay(3, SendsFrom::Transport)),
            Some(FetchTier::BrowserFetch),
            Some(FetchTier::Http),
            Some(FetchTier::Browser),
        )
    );
}

/// Under `fetch.http_from: learned`, a Label learned onto
/// `discovered_api` keeps the client its samples' replays matched from
/// (#2420): its own transport when every one matched there, the browser
/// when one matched only from there, and its later requests replay the
/// last plan from that client, drawing a Session that sends from it. A
/// resumed crawl replays from the same client.
///
/// Covers: setting:fetch.http_from, story:5-119, story:6-257
#[test]
fn a_label_learned_onto_discovered_api_replays_from_the_cheapest_client_that_matched_every_sample()
{
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
        connection.execute_batch(statement).unwrap();
    }
    let mut choices = TierChoices::default();
    for page in 1..=SAMPLES {
        let number = u32::try_from(page).unwrap();
        let mixed = if page == 2 {
            SendsFrom::Browser
        } else {
            SendsFrom::Transport
        };
        for (label, from) in [
            ("scroll", SendsFrom::Transport),
            ("guarded", SendsFrom::Browser),
            ("mixed", mixed),
        ] {
            weigh_from(
                &mut choices,
                label,
                &format!("/{label}/{page}"),
                HttpFrom::Learned,
                Weighed::Differed(Between {
                    api: Some(replay(number, from)),
                    ..Between::default()
                }),
            );
        }
    }
    let plans = |choices: &mut TierChoices| {
        ["scroll", "guarded", "mixed"].map(|label| {
            plan_from(
                choices,
                label,
                &format!("/{label}/9"),
                HttpFrom::Learned,
                START,
            )
        })
    };
    let learned = plans(&mut choices);
    for ((label, key), unit) in &choices.units {
        record(&connection, label, key, unit).unwrap();
    }
    let resumed = plans(&mut read(&connection).unwrap());

    let expected = [
        replay(3, SendsFrom::Transport),
        replay(3, SendsFrom::Browser),
        replay(3, SendsFrom::Browser),
    ]
    .map(AttemptPlan::Replayed);
    assert_eq!(learned, expected);
    assert_eq!(resumed, expected);
    assert_eq!(
        expected
            .each_ref()
            .map(|plan| plan.sends_from(SendsFrom::Transport)),
        [SendsFrom::Transport, SendsFrom::Browser, SendsFrom::Browser]
    );
}

/// Only a sample under `fetch.http_from: learned` runs on
/// `browser_fetch`, its `http` run sent from the transport; a choice that
/// holds `browser_fetch` is sampled again once its Label sends its HTTP
/// from the transport alone (#2417).
///
/// Covers: setting:fetch.http_from
#[test]
fn only_a_sample_under_learned_runs_on_browser_fetch() {
    let mut choices = TierChoices::default();
    let sampled = [HttpFrom::Browser, HttpFrom::Learned, HttpFrom::Transport].map(|from| {
        match plan_from(&mut choices, "page", "/identity/1", from, START) {
            AttemptPlan::Sample(sampled) => (sampled.from, sampled.browser_fetch),
            other => panic!("{other:?} is no sample"),
        }
    });
    for page in 1..=SAMPLES {
        weigh_from(
            &mut choices,
            "identity",
            &format!("/identity/{page}"),
            HttpFrom::Learned,
            fetched(),
        );
    }
    let learned = tier_from(&mut choices, "identity", "/identity/9", HttpFrom::Learned);

    assert_eq!(
        (
            sampled,
            learned,
            tier_from(
                &mut choices,
                "identity",
                "/identity/10",
                HttpFrom::Transport
            ),
        ),
        (
            [
                (SendsFrom::Browser, false),
                (SendsFrom::Transport, true),
                (SendsFrom::Transport, false),
            ],
            Some(FetchTier::BrowserFetch),
            None,
        )
    );
}

/// A refusal of a Label's transport drops its choice (#2419): the Label
/// learned onto `http` from the transport is sampled again at once, a
/// resumed crawl too, and samples whose `http` run was refused but whose
/// `browser_fetch` run matched move it to `browser_fetch`. A Label still
/// sampling keeps the samples it took, and a walled Label stays walled.
///
/// Covers: setting:fetch.http_from, story:6-251
#[test]
fn a_refused_transport_samples_its_label_again_onto_browser_fetch() {
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
        connection.execute_batch(statement).unwrap();
    }
    let mut choices = TierChoices::default();
    for page in 1..=SAMPLES {
        weigh_from(
            &mut choices,
            "refused",
            &format!("/refused/{page}"),
            HttpFrom::Learned,
            Weighed::Matched,
        );
        if page < SAMPLES {
            weigh_from(
                &mut choices,
                "sampling",
                &format!("/refused/{page}"),
                HttpFrom::Learned,
                Weighed::Matched,
            );
        }
    }
    weigh(&mut choices, "walled", "/refused/1", Weighed::Walled);
    let before = tier_from(&mut choices, "refused", "/refused/4", HttpFrom::Learned);
    let dropped = choices.invalidate("refused", KEY).cloned();
    if let Some(unit) = &dropped {
        record(&connection, "refused", KEY, unit).unwrap();
    }
    let unchanged = [
        choices.invalidate("sampling", KEY).is_none(),
        choices.invalidate("walled", KEY).is_none(),
    ];
    let resumed = tier_from(
        &mut read(&connection).unwrap(),
        "refused",
        "/refused/4",
        HttpFrom::Learned,
    );
    for page in 4..4 + SAMPLES {
        weigh_from(
            &mut choices,
            "refused",
            &format!("/refused/{page}"),
            HttpFrom::Learned,
            fetched(),
        );
    }
    weigh_from(
        &mut choices,
        "sampling",
        "/refused/3",
        HttpFrom::Learned,
        Weighed::Matched,
    );

    assert_eq!(
        (
            before,
            dropped,
            unchanged,
            resumed,
            tier_from(&mut choices, "refused", "/refused/9", HttpFrom::Learned),
            tier_from(&mut choices, "sampling", "/refused/9", HttpFrom::Learned),
            tier(&mut choices, "walled", "/refused/9"),
        ),
        (
            Some(FetchTier::Http),
            Some(Unit::Sampling {
                samples: Vec::new(),
                replay: None,
                from: SendsFrom::Transport,
            }),
            [true, true],
            None,
            Some(FetchTier::BrowserFetch),
            Some(FetchTier::Http),
            Some(FetchTier::Browser),
        )
    );
}

/// A resumed crawl takes its `browser_fetch` choices again: a Label
/// learned onto it, and a split Label's segment, read back from the run
/// file on it, and so does a segment still sampling (#2417).
///
/// Covers: setting:fetch.http_from, story:6-257
#[test]
fn a_resumed_crawl_takes_its_browser_fetch_choices_again() {
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
        connection.execute_batch(statement).unwrap();
    }
    let mut choices = TierChoices::default();
    for page in 1..=SAMPLES {
        weigh_from(
            &mut choices,
            "identity",
            &format!("/identity/{page}"),
            HttpFrom::Learned,
            fetched(),
        );
    }
    weigh_from(
        &mut choices,
        "mixed",
        "/identity/1",
        HttpFrom::Learned,
        fetched(),
    );
    weigh_from(
        &mut choices,
        "mixed",
        "/scripted/1",
        HttpFrom::Learned,
        differed(),
    );
    weigh_from(
        &mut choices,
        "mixed",
        "/static/1",
        HttpFrom::Learned,
        fetched(),
    );
    for page in 2..=SAMPLES {
        weigh_from(
            &mut choices,
            "mixed",
            &format!("/identity/{page}"),
            HttpFrom::Learned,
            fetched(),
        );
    }
    for ((label, key), unit) in &choices.units {
        record(&connection, label, key, unit).unwrap();
    }

    let mut read = read(&connection).unwrap();
    for page in 2..=SAMPLES {
        weigh_from(
            &mut read,
            "mixed",
            &format!("/static/{page}"),
            HttpFrom::Learned,
            fetched(),
        );
    }

    assert_eq!(
        [
            tier_from(&mut read, "identity", "/identity/9", HttpFrom::Learned),
            tier_from(&mut read, "mixed", "/identity/9", HttpFrom::Learned),
            tier_from(&mut read, "mixed", "/static/9", HttpFrom::Learned),
            tier_from(&mut read, "mixed", "/scripted/9", HttpFrom::Learned),
        ],
        [
            Some(FetchTier::BrowserFetch),
            Some(FetchTier::BrowserFetch),
            Some(FetchTier::BrowserFetch),
            Some(FetchTier::Browser),
        ]
    );
}

/// A choice records the client its samples' HTTP was sent from: a Label
/// whose `fetch.http_from` no longer sends from it is sampled again, from
/// the client it now sends from, and the choice those samples make
/// stands for that client only. A walled Label stays on `browser` from
/// either (ADR 0048).
///
/// Covers: setting:fetch.http_from
#[test]
fn a_choice_learned_from_another_client_is_sampled_again() {
    let mut choices = TierChoices::default();
    for page in 1..=SAMPLES {
        sample(&mut choices, "api", &format!("/products/{page}"), true);
    }
    weigh(&mut choices, "walled", "/products/1", Weighed::Walled);
    let from_the_browser = tier(&mut choices, "api", "/products/9");
    let resampled = plan_from(
        &mut choices,
        "api",
        "/products/10",
        HttpFrom::Transport,
        START,
    );
    for page in 1..=SAMPLES {
        weigh_from(
            &mut choices,
            "api",
            &format!("/products/{page}"),
            HttpFrom::Transport,
            Weighed::Matched,
        );
    }

    assert_eq!(
        (
            from_the_browser,
            resampled,
            tier_from(&mut choices, "api", "/products/11", HttpFrom::Transport),
            tier(&mut choices, "api", "/products/12"),
            tier_from(&mut choices, "walled", "/products/2", HttpFrom::Transport),
        ),
        (
            Some(FetchTier::Http),
            AttemptPlan::Sample(Sampled {
                label: "api".to_owned(),
                domain_key: KEY.to_owned(),
                segment: "products".to_owned(),
                from: SendsFrom::Transport,
                browser_fetch: false,
            }),
            Some(FetchTier::Http),
            None,
            Some(FetchTier::Browser),
        )
    );
}

/// A forced `fetch.tier` ignores `fetch.http_from`: its Session sends
/// from the client its tier names, from either setting; under `auto`, a
/// Label learned onto `http` sends from the client the setting names,
/// and a sample's attempt from the browser, whose `http` run alone takes
/// the transport's Session.
///
/// Covers: setting:fetch.http_from
#[test]
fn a_forced_tier_sends_from_its_own_client_whatever_the_label_sends_its_http_from() {
    let forced = |tier| {
        AttemptPlan::On(TierChoice {
            tier,
            forced: Some(Forced { label: None }),
        })
    };
    let learned = AttemptPlan::On(TierChoice {
        tier: FetchTier::Http,
        forced: None,
    });
    let sample = AttemptPlan::Sample(Sampled {
        label: "api".to_owned(),
        domain_key: KEY.to_owned(),
        segment: "products".to_owned(),
        from: SendsFrom::Transport,
        browser_fetch: false,
    });
    let from_either = |plan: &AttemptPlan| {
        [SendsFrom::Browser, SendsFrom::Transport].map(|from| plan.sends_from(from))
    };

    assert_eq!(
        [
            from_either(&forced(FetchTier::BrowserFetch)),
            from_either(&forced(FetchTier::Http)),
            from_either(&learned),
            from_either(&sample),
        ],
        [
            [SendsFrom::Browser; 2],
            [SendsFrom::Transport; 2],
            [SendsFrom::Browser, SendsFrom::Transport],
            [SendsFrom::Browser; 2],
        ]
    );
}

/// A choice stored before choices kept their client reads back as learned
/// from the browser, a split one with or without its segments' `serves`,
/// and one learned from the browser is stored again with no client, as
/// before; one learned from the transport keeps its client through the run
/// file.
///
/// Covers: setting:fetch.http_from, story:6-257
#[test]
fn a_choice_stored_without_its_client_reads_back_as_sent_from_the_browser() {
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
        connection.execute_batch(statement).unwrap();
    }
    let learned = format!(r#"{{"kind":"learned","tier":"http","since":{START}}}"#);
    let split = format!(
        r#"{{"kind":"split","segments":[{{"name":"products","matched":3,"serves":{{"http":true,"api":true,"fetch":true,"light":true}},"tier":"http"}},{{"name":"docs","matched":3,"tier":"http"}},{{"name":"blog","matched":0,"tier":"browser"}}],"since":{START}}}"#
    );
    let sampling = r#"{"kind":"sampling","samples":[{"segment":"products","serves":{"http":true,"api":true,"fetch":true,"light":true}}]}"#;
    // A segment stored without its `serves` is stored again with them all.
    let split_again = split
        .replace(
            r#""matched":3,"tier""#,
            r#""matched":3,"serves":{"http":true,"api":true,"fetch":true,"light":true},"tier""#,
        )
        .replace(
            r#""matched":0,"tier""#,
            r#""matched":0,"serves":{"http":true,"api":true,"fetch":true,"light":true},"tier""#,
        );
    for (label, unit) in [
        ("learned", learned.as_str()),
        ("split", &split),
        ("sampling", sampling),
    ] {
        connection
            .execute(
                "INSERT INTO tier_choices (label, domain_key, unit) VALUES (?1, ?2, ?3)",
                (label, KEY, unit),
            )
            .unwrap();
    }
    let mut transport = TierChoices::default();
    for page in 1..=SAMPLES {
        weigh_from(
            &mut transport,
            "transport",
            &format!("/products/{page}"),
            HttpFrom::Transport,
            Weighed::Matched,
        );
    }
    for ((label, key), unit) in &transport.units {
        record(&connection, label, key, unit).unwrap();
    }

    let mut read = read(&connection).unwrap();
    let stored_again =
        ["learned", "split"].map(|label| encoded(&read.units[&(label.to_owned(), KEY.to_owned())]));
    let sampling_from_the_browser = (
        tier(&mut read, "sampling", "/products/2"),
        weigh(&mut read, "sampling", "/products/2", Weighed::Matched),
        weigh(&mut read, "sampling", "/products/3", Weighed::Matched),
        tier(&mut read, "sampling", "/products/4"),
    );

    assert_eq!(
        (
            stored_again,
            tier(&mut read, "learned", "/products/9"),
            tier(&mut read, "split", "/products/9"),
            tier(&mut read, "split", "/docs/9"),
            tier(&mut read, "split", "/blog/9"),
            sampling_from_the_browser,
            tier_from(&mut read, "transport", "/products/9", HttpFrom::Transport),
            tier(&mut read, "transport", "/products/10"),
        ),
        (
            [learned, split_again],
            Some(FetchTier::Http),
            Some(FetchTier::Http),
            Some(FetchTier::Http),
            Some(FetchTier::Browser),
            (None, (), (), Some(FetchTier::Http)),
            Some(FetchTier::Http),
            None,
        )
    );
}
