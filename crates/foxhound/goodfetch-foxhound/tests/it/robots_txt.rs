//! Politeness conformance cases for the shared robots.txt copy (spec
//! #581's seam: the protocol against the Proving ground; ADR 0086): crawls
//! under `fetch.tier: http` on Foxhound's executable, each in an engine of
//! its own under one Goodfetch home, against the `access-policy` family's
//! hosts, whose robots.txt the Proving ground counts. A crawl's Run reads
//! each origin's robots.txt once for all its pages, from the copy the
//! user's runs share in the Politeness file, never from a copy of its own;
//! a run within the day sends no robots.txt request, and one after it
//! does; and the fetch is a request of its domain, the delay between starts
//! counted from it. A host's Crawl-delay spaces every sharing run's
//! requests to its domain key, and one above
//! `site_preferences.max_crawl_delay` skips the host's requests.
#![expect(
    clippy::disallowed_methods,
    reason = "the test measures, from outside the engine's process, how long its first page waited, and writes the Operator policy itself"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::{Duration, Instant};

use goodfetch_protocol::home::POLITENESS_FILE;
use goodfetch_test_client::Attached;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::ObservedRequest;
use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};

use crate::attached;
use crate::attached_with;
use crate::backoff_classes::held_for;
use crate::frontier::crawl_with;

/// The Proving ground's seed.
const SEED: u64 = 877;

/// The `access-policy` family's first task's host, whose robots.txt
/// answers `200` with a file.
const FILE_HOST: &str = "1.access-policy.test";

/// A day, in milliseconds.
const DAY: i64 = 86_400_000;

/// The settings of a crawl that fetches its pages on `host` of
/// `proving_ground`, with `more` merged in.
fn settings(proving_ground: &ProvingGround, host: &str, more: &Value) -> Value {
    let ip = proving_ground.address().ip().to_string();
    let mut settings = json!({
        "fetch": { "tier": "http" },
        "network": { "resolve": { host: [ip] } },
        "egress": { "private_ranges": [host] },
    });
    if let (Some(settings), Some(more)) = (settings.as_object_mut(), more.as_object()) {
        settings.extend(more.clone());
    }
    settings
}

/// Crawls `paths` of `host` as `run_id` in an engine of its own under
/// `env`'s home, its handler committing nothing; answers the Run report.
fn crawl(
    env: &TestEnv,
    proving_ground: &ProvingGround,
    run_id: &str,
    host: &str,
    paths: &[&str],
) -> Value {
    let seeds: Vec<Value> = paths
        .iter()
        .map(|path| json!({ "url": proving_ground.url(host, path), "label": "page" }))
        .collect();
    let mut engine = attached(env);
    let report = crawl_with(
        &mut engine,
        run_id,
        &seeds,
        &settings(proving_ground, host, &json!({})),
        &mut |_, _| json!({ "result": { "records": [] } }),
    );
    assert!(engine.finished());
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    report
}

/// How many requests `host` received so far, for its robots.txt and for
/// its other pages.
fn received(proving_ground: &ProvingGround, host: &str) -> (usize, usize) {
    let observations = proving_ground.observations().unwrap();
    let received = ReceivedRequests::of(&observations);
    let requests: Vec<&ObservedRequest> = received
        .every()
        .filter(|request| request.host == host)
        .collect();
    let robots = requests
        .iter()
        .filter(|request| request.target == "/robots.txt")
        .count();
    (robots, requests.len() - robots)
}

/// Moves every copy in the home's Politeness file a day into the past, as
/// if it had been fetched a day earlier.
fn age_the_copies(env: &TestEnv) {
    let file = rusqlite::Connection::open(env.home().join("politeness.sqlite")).unwrap();
    file.execute("UPDATE robots_txt SET fetched_at = fetched_at - ?1", [DAY])
        .unwrap();
}

/// A crawl of two pages fetches its origin's robots.txt once; a second
/// run, in another engine within the day, sends no robots.txt request,
/// since it reads the copy the first left in the Politeness file rather
/// than a copy of its own; once the copy is a day old, a third sends one.
///
/// Covers: story:6-241, story:6-246, setting:fetch.tier, format:politeness-file
#[test]
fn a_crawl_reads_robots_txt_once_a_day_from_the_copy_its_users_runs_share() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();

    crawl(
        &env,
        &proving_ground,
        "first",
        FILE_HOST,
        &["/catalog/1", "/catalog/2"],
    );
    let after_first = received(&proving_ground, FILE_HOST);
    crawl(&env, &proving_ground, "second", FILE_HOST, &["/catalog/3"]);
    let after_second = received(&proving_ground, FILE_HOST);
    age_the_copies(&env);
    crawl(&env, &proving_ground, "third", FILE_HOST, &["/catalog/1"]);
    let after_third = received(&proving_ground, FILE_HOST);

    assert_eq!(
        (after_first, after_second, after_third),
        ((1, 2), (1, 3), (2, 4))
    );
}

/// The `access-policy` family's second task's host, whose robots.txt
/// answers `503`.
const UNREACHABLE_HOST: &str = "2.access-policy.test";

/// The fetch edge pages' host.
const EDGE_HOST: &str = "fetch-edge.test";

/// The named host whose robots.txt has groups for `Goodfetch-Foxhound`
/// (`/foxhound/`), the operator token `operator-bot` (`/operator/`), a bare
/// `Foxhound` (`/bare/`) and `*` (`/star/`).
const TOKENS_HOST: &str = "tokens.access-policy.test";

/// The named host whose robots.txt has groups for a bare `Foxhound`
/// (`/bare/`) and `*` (`/star/`) only.
const STAR_HOST: &str = "star.access-policy.test";

/// What a crawl did: its Run report, the path of each request its handler
/// was given, and each outcome's path, kind and skip reason, in commit
/// order.
struct Crawled {
    report: Value,
    handled: Vec<String>,
    outcomes: Vec<(String, Value, Value)>,
}

/// The path of `url`.
fn path_of(url: &Value) -> String {
    let url = url.as_str().unwrap();
    let rest = url.split_once("://").unwrap().1;
    rest[rest.find('/').unwrap()..].to_owned()
}

/// Crawls `seeds`, each a host and a path, as `run_id` under `fetch.tier:
/// http` and `more` settings, each host resolved to the Proving ground;
/// the handler enqueues what `links` gives for each page and commits
/// nothing.
fn crawl_seeds(
    env: &TestEnv,
    proving_ground: &ProvingGround,
    run_id: &str,
    seeds: &[(&str, &str)],
    more: &Value,
    links: &dyn Fn(&str) -> Vec<String>,
) -> Crawled {
    let ip = proving_ground.address().ip().to_string();
    let mut resolve = serde_json::Map::new();
    for (host, _) in seeds {
        resolve.insert((*host).to_owned(), json!([ip]));
    }
    let mut settings = json!({
        "fetch": { "tier": "http" },
        "egress": { "private_ranges": resolve.keys().collect::<Vec<_>>() },
        "network": { "resolve": resolve },
    });
    if let (Some(settings), Some(more)) = (settings.as_object_mut(), more.as_object()) {
        settings.extend(more.clone());
    }
    let seeds: Vec<Value> = seeds
        .iter()
        .map(|(host, path)| json!({ "url": proving_ground.url(host, path), "label": "page" }))
        .collect();
    let mut engine = attached(env);
    let mut handled = Vec::new();
    let report = crawl_with(
        &mut engine,
        run_id,
        &seeds,
        &settings,
        &mut |inside, handle| {
            let url = &handle["crawl_request"]["url"];
            handled.push(path_of(url));
            for link in links(&path_of(url)) {
                inside.enqueue(handle, &link, "page", None);
            }
            json!({ "result": { "records": [] } })
        },
    );
    let read = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    assert!(engine.finished());
    let outcomes = read["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("{read}"))
        .iter()
        .map(|outcome| {
            (
                path_of(&outcome["request"]["url"]),
                outcome["kind"].clone(),
                outcome["reason"].clone(),
            )
        })
        .collect();
    Crawled {
        report,
        handled,
        outcomes,
    }
}

/// The paths `host` received, robots.txt left out.
fn pages_received(proving_ground: &ProvingGround, host: &str) -> Vec<String> {
    let observations = proving_ground.observations().unwrap();
    ReceivedRequests::of(&observations)
        .of_the_case()
        .filter(|request| request.host == host)
        .map(|request| request.target.as_str().to_owned())
        .collect()
}

fn ended(path: &str, kind: &str, reason: Option<&str>) -> (String, Value, Value) {
    (
        path.to_owned(),
        json!(kind),
        reason.map_or(Value::Null, |reason| json!(reason)),
    )
}

/// A start URL and a discovered link that robots.txt disallows to
/// `goodfetch-foxhound` each end `skipped` with `robots_txt`: neither is
/// sent, and no handler is run for either.
///
/// Covers: story:6-239, setting:site_preferences.respect_robots_txt
#[test]
fn a_start_url_and_a_discovered_link_robots_txt_disallows_end_skipped_unsent() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();

    let crawled = crawl_seeds(
        &env,
        &proving_ground,
        "disallowed",
        &[(FILE_HOST, "/catalog/1"), (FILE_HOST, "/private/catalog/1")],
        &json!({}),
        &|path| {
            if path == "/catalog/1" {
                vec!["/private/catalog/2".to_owned()]
            } else {
                Vec::new()
            }
        },
    );
    let mut outcomes = crawled.outcomes;
    outcomes.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(
        (
            &crawled.report["result"]["lifecycle"],
            crawled.handled,
            outcomes,
            pages_received(&proving_ground, FILE_HOST),
        ),
        (
            &json!("finished"),
            vec!["/catalog/1".to_owned()],
            vec![
                ended("/catalog/1", "succeeded", None),
                ended("/private/catalog/1", "skipped", Some("robots_txt")),
                ended("/private/catalog/2", "skipped", Some("robots_txt")),
            ],
            vec!["/catalog/1".to_owned()],
        )
    );
}

/// A crawl request obeys the robots.txt groups for `goodfetch-foxhound`
/// and the operator's token merged, and the `*` group only where neither
/// has one; a bare `Foxhound` group is no group of the crawl's.
///
/// Covers: story:6-239, setting:site_preferences.robots_txt_user_agent
#[test]
fn a_crawl_request_obeys_foxhounds_group_merged_with_the_operators_before_stars() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();

    let crawled = crawl_seeds(
        &env,
        &proving_ground,
        "tokens",
        &[
            (TOKENS_HOST, "/foxhound/1"),
            (TOKENS_HOST, "/operator/1"),
            (TOKENS_HOST, "/bare/1"),
            (TOKENS_HOST, "/star/1"),
            (STAR_HOST, "/bare/2"),
            (STAR_HOST, "/star/2"),
        ],
        &json!({ "site_preferences": { "robots_txt_user_agent": "operator-bot" } }),
        &|_| Vec::new(),
    );
    let mut outcomes = crawled.outcomes;
    outcomes.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(
        outcomes,
        vec![
            ended("/bare/1", "succeeded", None),
            ended("/bare/2", "succeeded", None),
            ended("/foxhound/1", "skipped", Some("robots_txt")),
            ended("/operator/1", "skipped", Some("robots_txt")),
            ended("/star/1", "succeeded", None),
            ended("/star/2", "skipped", Some("robots_txt")),
        ]
    );
}

/// A request waiting on a robots.txt that keeps answering `503`, with no
/// earlier copy, is never sent: each failed robots.txt fetch counts as one
/// of its attempts, and once its retries ran out it ends `skipped` with
/// `robots_txt`, so it never waits forever.
///
/// Covers: story:6-242, setting:crawl.retry.max
#[test]
fn a_request_whose_robots_txt_keeps_failing_ends_skipped_after_its_retries() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();

    let crawled = crawl_seeds(
        &env,
        &proving_ground,
        "unreachable",
        &[(UNREACHABLE_HOST, "/catalog/1")],
        &json!({
            "crawl": { "retry": { "max": 1 } },
            "politeness": { "backoff": { "initial": 100, "max": 200 } },
        }),
        &|_| Vec::new(),
    );

    assert_eq!(
        (
            &crawled.report["result"]["lifecycle"],
            crawled.handled.len(),
            crawled.outcomes,
            received(&proving_ground, UNREACHABLE_HOST),
        ),
        (
            &json!("finished"),
            0,
            vec![ended("/catalog/1", "skipped", Some("robots_txt"))],
            (2, 0),
        )
    );
}

/// The `fetch-edge.test` page that redirects to a page of
/// [`UNREACHABLE_HOST`].
const REDIRECT_TO_UNREACHABLE: &str = "/redirect/unreachable-robots";

/// A redirect hop whose host's robots.txt keeps answering `503`, with no
/// earlier copy, is never sent, and its crawl request is retried as one
/// whose own robots.txt failed: each failed robots.txt fetch counts as one
/// of its attempts, and once its retries ran out it ends `skipped` with
/// `robots_txt`. The crawl's scope takes every host, so the hop leaves the
/// seed's domain.
///
/// Covers: story:6-242, setting:crawl.retry.max, setting:crawl.scope.strategy
#[test]
fn a_request_whose_redirect_hops_robots_txt_keeps_failing_ends_skipped_after_its_retries() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let ip = proving_ground.address().ip().to_string();

    let crawled = crawl_seeds(
        &env,
        &proving_ground,
        "unreachable-hop",
        &[(EDGE_HOST, REDIRECT_TO_UNREACHABLE)],
        &json!({
            "crawl": { "retry": { "max": 1 }, "scope": { "strategy": "all" } },
            "politeness": { "backoff": { "initial": 100, "max": 200 } },
            "network": { "resolve": { EDGE_HOST: [&ip], UNREACHABLE_HOST: [&ip] } },
            "egress": { "private_ranges": [EDGE_HOST, UNREACHABLE_HOST] },
        }),
        &|_| Vec::new(),
    );

    assert_eq!(
        (
            crawled.handled.len(),
            crawled.outcomes,
            received(&proving_ground, UNREACHABLE_HOST),
        ),
        (
            0,
            vec![ended(
                REDIRECT_TO_UNREACHABLE,
                "skipped",
                Some("robots_txt")
            )],
            (2, 0),
        )
    );
}

/// A robots.txt that answers `503`, with no earlier copy, backs off the
/// domain whose file it is when that is the request's own: a redirect hop's
/// says nothing of the healthy domain that redirected there, which the
/// crawl may go on asking.
///
/// Covers: story:6-242, setting:politeness.backoff.initial, format:politeness-file
#[rstest]
#[case::its_own(UNREACHABLE_HOST, "/catalog/1", "access-policy.test", true)]
#[case::a_redirect_hops(EDGE_HOST, REDIRECT_TO_UNREACHABLE, EDGE_HOST, false)]
fn only_a_requests_own_unreachable_robots_txt_backs_its_domain_off(
    #[case] host: &str,
    #[case] path: &str,
    #[case] domain_key: &str,
    #[case] backs_off: bool,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let ip = proving_ground.address().ip().to_string();

    let crawled = crawl_seeds(
        &env,
        &proving_ground,
        "unreachable-domain",
        &[(host, path)],
        &json!({
            "crawl": { "retry": { "max": 0 }, "scope": { "strategy": "all" } },
            "politeness": { "backoff": { "initial": 3_600_000, "max": 3_600_000 } },
            "network": { "resolve": { EDGE_HOST: [&ip], UNREACHABLE_HOST: [&ip] } },
            "egress": { "private_ranges": [EDGE_HOST, UNREACHABLE_HOST] },
        }),
        &|_| Vec::new(),
    );

    assert_eq!(
        (
            crawled.outcomes,
            held_for(
                &env.home().join(POLITENESS_FILE),
                domain_key,
                Duration::from_mins(30)
            ),
        ),
        (vec![ended(path, "skipped", Some("robots_txt"))], backs_off)
    );
}

/// A request robots.txt disallows is refused as soon as its robots.txt is
/// read, before any wait for its domain's delay after that robots.txt: it
/// takes no start of its domain, so a request it never sent holds back no
/// other.
///
/// Covers: story:6-239, story:6-241, setting:politeness.domain.delay, format:politeness-file
#[test]
fn a_request_robots_txt_disallows_takes_no_start_of_its_domain() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();

    let crawled = crawl_seeds(
        &env,
        &proving_ground,
        "disallowed-unstarted",
        &[(FILE_HOST, "/private/catalog/1")],
        &json!({ "politeness": { "domain": { "delay": 2_000 } } }),
        &|_| Vec::new(),
    );
    let file = rusqlite::Connection::open(env.home().join(POLITENESS_FILE)).unwrap();
    let (last_start, fetched_at): (i64, i64) = file
        .query_row(
            "SELECT (SELECT MAX(last_start) FROM domain_limits), (SELECT MAX(fetched_at) FROM robots_txt)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    assert_eq!(
        (crawled.outcomes, last_start <= fetched_at),
        (
            vec![ended("/private/catalog/1", "skipped", Some("robots_txt"))],
            true
        ),
        "the domain's last start {last_start}, its robots.txt fetched at {fetched_at}"
    );
}

/// With `site_preferences.respect_robots_txt` off, a request robots.txt
/// disallows is fetched and handled, and robots.txt itself is never
/// fetched: off costs nothing (ADR 0138).
///
/// Covers: story:6-239, setting:site_preferences.respect_robots_txt
#[test]
fn with_robots_txt_not_respected_a_disallowed_request_is_fetched() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();

    let crawled = crawl_seeds(
        &env,
        &proving_ground,
        "ignored",
        &[(FILE_HOST, "/private/catalog/1")],
        &json!({ "site_preferences": { "respect_robots_txt": false } }),
        &|_| Vec::new(),
    );

    assert_eq!(
        (
            crawled.handled,
            crawled.outcomes,
            pages_received(&proving_ground, FILE_HOST),
            received(&proving_ground, FILE_HOST),
        ),
        (
            vec!["/private/catalog/1".to_owned()],
            vec![ended("/private/catalog/1", "succeeded", None)],
            vec!["/private/catalog/1".to_owned()],
            (0, 1),
        )
    );
}

/// An Operator policy that locks `site_preferences.respect_robots_txt` on
/// is honoured (ADR 0028): a crawl that turns it off is refused at its
/// start with `setting_locked`, and the site gets nothing.
///
/// Covers: story:6-239, story:6-314, setting:site_preferences.respect_robots_txt, error:setting_locked
#[test]
fn an_operator_policy_lock_keeps_robots_txt_respected() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let layout = HomeLayout::of(env.home());
    std::fs::create_dir_all(layout.per_major()).unwrap();
    std::fs::write(
        layout.operator_policy(),
        "version = \"0.1\"\n\n[lock.site_preferences]\nrespect_robots_txt = true\n",
    )
    .unwrap();
    let mut engine = attached(&env);

    let started = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "locked",
            "seeds": [{ "url": proving_ground.url(FILE_HOST, "/private/catalog/1"), "label": "page" }],
            "settings": settings(
                &proving_ground,
                FILE_HOST,
                &json!({ "site_preferences": { "respect_robots_txt": false } }),
            ),
        }),
        &mut |_| None,
    );
    assert!(engine.finished());

    assert_eq!(
        (
            &started["error"]["data"]["code"],
            pages_received(&proving_ground, FILE_HOST),
        ),
        (&json!("setting_locked"), Vec::<String>::new()),
        "{started}"
    );
}

/// The robots.txt fetch is a request of its domain: under a delay between
/// starts of 2 s, the crawl's first page goes no sooner than 2 s after its
/// origin's robots.txt.
///
/// Covers: story:6-241, setting:politeness.domain.delay
#[test]
fn the_robots_txt_fetch_counts_in_its_domains_politeness() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let delay = Duration::from_secs(2);
    let more = json!({ "politeness": { "domain": { "delay": 2_000 } } });
    let mut engine = attached(&env);

    let started = Instant::now();
    let mut first_page = None;
    let report = crawl_with(
        &mut engine,
        "delayed",
        &[json!({ "url": proving_ground.url(FILE_HOST, "/catalog/1"), "label": "page" })],
        &settings(&proving_ground, FILE_HOST, &more),
        &mut |_, _| {
            first_page.get_or_insert_with(|| started.elapsed());
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            received(&proving_ground, FILE_HOST),
            first_page.is_some_and(|after| after >= delay),
        ),
        (&json!("finished"), (1, 1), true),
        "the first page came {first_page:?} after the start"
    );
}

/// A document waiting for its domain's delay after the robots.txt its
/// attempt fetched is not timed out meanwhile (ADR 0093): under a delay
/// longer than the attempt's lease was granted for, the lease is renewed
/// through the wait, and the request is fetched and handled in its first
/// attempt, which a crawl allowed no retry needs.
///
/// Covers: story:6-241, setting:politeness.domain.delay, setting:timeout.handler, setting:crawl.retry.max
#[test]
fn a_document_waiting_for_its_delay_after_robots_txt_keeps_its_lease() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    // The document waits out the delay with nothing to say.
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |_| {});
    // A lease of 1 s plus the 10 s margin, shorter than the delay.
    let more = json!({
        "timeout": { "handler": 1_000 },
        "politeness": { "domain": { "delay": 12_000 } },
        "crawl": { "retry": { "max": 0 } },
    });

    let mut handled = 0;
    let report = crawl_with(
        &mut engine,
        "waits",
        &[json!({ "url": proving_ground.url(FILE_HOST, "/catalog/1"), "label": "page" })],
        &settings(&proving_ground, FILE_HOST, &more),
        &mut |_, _| {
            handled += 1;
            json!({ "result": { "records": [] } })
        },
    );
    let outcomes = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "waits" }),
        &mut |_| None,
    );
    assert!(engine.finished());

    let outcome = &outcomes["result"]["outcomes"][0];
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            handled,
            (&outcome["kind"], &outcome["attempts"]),
            received(&proving_ground, FILE_HOST),
        ),
        (
            &json!("finished"),
            1,
            (&json!("succeeded"), &json!(1)),
            (1, 1)
        ),
        "{outcomes}"
    );
}

/// The named host whose robots.txt asks every crawler for a Crawl-delay of
/// 5 s.
const CRAWL_DELAY_HOST: &str = "crawl-delay.access-policy.test";

/// The Crawl-delay [`CRAWL_DELAY_HOST`] asks for, in microseconds.
const CRAWL_DELAY: u64 = 5_000_000;

/// Each request `host` received so far, robots.txt included: its target
/// and when it arrived, in microseconds, in arrival order.
pub(crate) fn arrivals(proving_ground: &ProvingGround, host: &str) -> Vec<(String, u64)> {
    let observations = proving_ground.observations().unwrap();
    let mut arrivals: Vec<(String, u64)> = ReceivedRequests::of(&observations)
        .every()
        .filter(|request| request.host == host)
        .map(|request| {
            (
                request.target.as_str().to_owned(),
                request.timing.arrived_microseconds,
            )
        })
        .collect();
    arrivals.sort_by_key(|(_, at)| *at);
    arrivals
}

/// Crawls two pages of `host`, `/<run>/1` and `/<run>/2`, as each of
/// `runs` at once, each in an engine of its own under `env`'s home, their
/// handlers committing nothing, until every Run finished.
fn crawl_at_once(env: &TestEnv, proving_ground: &ProvingGround, host: &str, runs: &[&str]) {
    let mut engines: Vec<(Attached, &str, bool)> = runs
        .iter()
        .map(|run| {
            let mut engine = attached(env);
            let mut settings = settings(proving_ground, host, &json!({}));
            settings["crawl"]["handler"] = json!({ "concurrency": 1 });
            let seeds: Vec<Value> = [1, 2]
                .iter()
                .map(|page| {
                    json!({ "url": proving_ground.url(host, &format!("/{run}/{page}")), "label": "page" })
                })
                .collect();
            engine.send(&json!({
                "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
                "params": { "run_id": run, "seeds": seeds, "settings": settings },
            }));
            (engine, *run, false)
        })
        .collect();
    while engines.iter().any(|(_, _, finished)| !finished) {
        for (engine, run, finished) in &mut engines {
            if *finished || engine.is_quiet_for(Duration::from_millis(50)) {
                continue;
            }
            let message = engine
                .next()
                .expect("the engine closed before its Run ended");
            if message["method"] == "crawl_request.handle" {
                engine.send(&json!({
                    "jsonrpc": "2.0", "id": message["id"], "result": { "records": [] },
                }));
            } else if message["id"] == "start" {
                assert_eq!(message["result"]["run_id"], *run, "{message}");
                engine.send(&json!({
                    "jsonrpc": "2.0", "id": "finish", "method": "run.finish",
                    "params": { "run_id": run },
                }));
            } else if message["id"] == "finish" {
                assert_eq!(message["result"]["lifecycle"], "finished", "{message}");
                *finished = true;
            }
        }
    }
    for (engine, _, _) in engines {
        assert!(engine.finished());
    }
}

/// Two runs crawling one domain key at once, each in an engine of its own
/// under one Goodfetch home and with no `politeness.domain.delay` of their
/// own, keep to its host's Crawl-delay of 5 s: the site receives no page,
/// by either run, sooner than 5 s after the page before it, and the first
/// no sooner than 5 s after the first robots.txt request. (Each run's
/// robots.txt fetch, sent before the Crawl-delay is known, may go beside
/// the other's.) A page's delay counts from the start of the page before
/// it, which the site sees only once its request arrived: the pages may
/// arrive a little closer than their starts went, never 9/10 of the delay
/// closer. The first page's delay counts from the robots.txt response's
/// arrival, so the site never receives it sooner than 5 s after the
/// robots.txt request.
///
/// Covers: story:6-243, setting:site_preferences.max_crawl_delay, format:politeness-file
#[test]
fn a_crawl_delay_spaces_two_concurrent_runs_on_one_domain_key() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();

    crawl_at_once(
        &env,
        &proving_ground,
        CRAWL_DELAY_HOST,
        &["first", "second"],
    );
    let arrivals = arrivals(&proving_ground, CRAWL_DELAY_HOST);
    let first_robots_txt = arrivals
        .iter()
        .find(|(target, _)| target == "/robots.txt")
        .map(|(_, at)| *at);
    let pages: Vec<u64> = arrivals
        .iter()
        .filter(|(target, _)| target != "/robots.txt")
        .map(|(_, at)| *at)
        .collect();
    let spaced = pages
        .windows(2)
        .all(|pair| pair[1].saturating_sub(pair[0]) >= CRAWL_DELAY * 9 / 10);
    let after_robots_txt = matches!(
        (first_robots_txt, pages.first()),
        (Some(robots), Some(page)) if page.saturating_sub(robots) >= CRAWL_DELAY
    );

    assert!(
        pages.len() == 4 && spaced && after_robots_txt,
        "{arrivals:?}"
    );
}

/// A host asking for a Crawl-delay above `site_preferences.max_crawl_delay`,
/// here 3 s, has its crawl requests skipped with `crawl_delay`: none is
/// sent and no handler runs, so the crawl neither waits the host out nor
/// ignores it.
///
/// Covers: story:6-243, setting:site_preferences.max_crawl_delay
#[test]
fn a_host_asking_for_more_than_the_max_crawl_delay_is_skipped_with_crawl_delay() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();

    let crawled = crawl_seeds(
        &env,
        &proving_ground,
        "too-slow",
        &[
            (CRAWL_DELAY_HOST, "/catalog/1"),
            (CRAWL_DELAY_HOST, "/catalog/2"),
        ],
        &json!({ "site_preferences": { "max_crawl_delay": 3_000 } }),
        &|_| Vec::new(),
    );
    let mut outcomes = crawled.outcomes;
    outcomes.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(
        (
            &crawled.report["result"]["lifecycle"],
            crawled.handled,
            outcomes,
            pages_received(&proving_ground, CRAWL_DELAY_HOST),
        ),
        (
            &json!("finished"),
            Vec::<String>::new(),
            vec![
                ended("/catalog/1", "skipped", Some("crawl_delay")),
                ended("/catalog/2", "skipped", Some("crawl_delay")),
            ],
            Vec::<String>::new(),
        )
    );
}
