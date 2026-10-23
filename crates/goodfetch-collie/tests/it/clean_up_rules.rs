//! The version of the Clean-up rules each Run holds to (spec #577, ticket
//! #714; ADR 0047), through Collie's executable: chosen at the Run's start
//! from the embedded part, the daily channel or the Filter lists' own URLs,
//! its digest recorded in run state and kept by a resume, named in the Run
//! trace's Provenance with its date and the release that applied it, and
//! warned of in the Run report when it is older than 14 days or not the one
//! asked for. The daily channel's registry and the lists' authors are
//! local HTTPS stand-ins that `network.resolve` maps their hosts to, their
//! certificate authority trusted through `tls.trusted_cas` in user
//! configuration: no test reaches the internet.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read and write the homes' files, and run the sources' stand-in on a thread, outside any engine (ADR 0062)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the sources' stand-in listens and keeps its table outside the engine (ADR 0060)"
)]

mod stand_in;

use std::path::{Path, PathBuf};

use goodfetch_protocol::formats::CLEAN_UP_RULES;
use goodfetch_protocol::page::PageOpened;
use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache};
use rusqlite::Connection;
use serde_json::{Map, Value, json};

use self::stand_in::{StandIn, channel, manifest_path};
use crate::attached::{Attached, hello};
use crate::page::{BROWSER_DEADLINE, opened};
use crate::run::call;

/// The digest of the embedded part, as the clean-up crate's build gives it.
const EMBEDDED: &str = goodfetch_clean_up_rules::rules::EMBEDDED_DIGEST;

/// The folder of the Run `run_id` in the Goodfetch home `home`.
fn run_folder(home: &Path, run_id: &str) -> PathBuf {
    home.join("0.0").join("runs").join(run_id)
}

/// The Clean-up rules digest the run state of `run_id` records.
fn recorded_digest(home: &Path, run_id: &str) -> Option<String> {
    Connection::open(run_folder(home, run_id).join("run.sqlite"))
        .unwrap()
        .query_row("SELECT clean_up_digest FROM run", [], |row| row.get(0))
        .unwrap()
}

/// The Provenance entries of the Run `run_id` that name its Clean-up rules.
fn rules_provenance(home: &Path, run_id: &str) -> Vec<Value> {
    std::fs::read_to_string(run_folder(home, run_id).join("trace/provenance.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["action"] == json!("clean_up_rules"))
        .collect()
}

/// Starts and finishes the Run `run_id` on a new Attached engine with
/// `settings`, and answers its Run report.
fn start_and_finish(env: &TestEnv, run_id: &str, settings: &Value) -> Value {
    let mut engine = Attached::start(env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    let started = call(
        &mut engine,
        1,
        "run.start",
        &json!({ "run_id": run_id, "settings": settings }),
    );
    assert!(started.get("result").is_some(), "{started}");
    let report = call(&mut engine, 2, "run.finish", &json!({ "run_id": run_id }));
    let _ = engine.finish();
    report["result"].clone()
}

/// The codes of the warnings of the Run report `report`.
fn warning_codes(report: &Value) -> Vec<String> {
    report["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|warning| warning["code"].as_str().map(str::to_owned))
        .collect()
}

/// The date of the embedded part, as its manifest gives it.
fn embedded_date() -> Value {
    serde_json::from_str::<Value>(goodfetch_clean_up_rules::rules::EMBEDDED_MANIFEST).unwrap()
        ["published_at"]
        .clone()
}

/// Covers: story:5-97, setting:clean_up.update
#[test]
fn a_run_holds_to_the_embedded_part_by_default_named_with_its_date_and_release() {
    let env = TestEnv::new();

    let report = start_and_finish(&env, "embedded", &json!({}));

    assert_eq!(
        (
            recorded_digest(env.home(), "embedded"),
            rules_provenance(env.home(), "embedded"),
            warning_codes(&report).contains(&"clean_up_rules_fallback".to_owned()),
        ),
        (
            Some(EMBEDDED.to_owned()),
            vec![json!({
                "action": "clean_up_rules",
                "digest": EMBEDDED,
                "source": "embedded",
                "published_at": embedded_date(),
                "applied_by": { "product": "collie", "product_version": "0.0.0" },
                "update": "release",
                "resumed": false,
            })],
            false,
        )
    );
}

/// The embedded part's digest is the Browser cache's digest of its files,
/// so a copy of them that a release publishes is checked as any download
/// is (ADR 0024).
/// Covers: story:5-97
#[test]
fn the_embedded_parts_digest_is_the_browser_caches_digest_of_its_files() {
    let rules = Path::new(env!("CARGO_MANIFEST_DIR")).join("../goodfetch-clean-up-rules/rules");
    let copy = tempfile::tempdir().unwrap();
    for name in [
        "manifest.json",
        "easylist.txt",
        "easyprivacy.txt",
        "fanboy-annoyance.txt",
        "brave-unbreak.txt",
    ] {
        std::fs::copy(rules.join(name), copy.path().join(name)).unwrap();
    }

    assert_eq!(
        goodfetch_engine::browser_cache::content_digest(copy.path())
            .unwrap()
            .to_string(),
        EMBEDDED
    );
}

/// The stand-in daily channel's registry.
const REGISTRY: &str = "registry.test";

/// The stand-in lists' authors.
const LISTS: &str = "lists.test";

/// The Proving ground's seed.
const SEED: u64 = 714;

/// The `/ads` page of `clean-up.test`'s ad, and its tracker.
const AD: &str = "/img/top-ad-sidebar.gif";
const TRACKER: &str = "/js/adobe-analytics.js";

/// The daily channel's tag this engine pulls: its release line and the
/// manifest format's line, each `0.y` minor counting as a major.
fn channel_tag() -> String {
    let mut product = env!("CARGO_PKG_VERSION").split('.');
    let (major, minor) = (product.next().unwrap(), product.next().unwrap());
    let format = CLEAN_UP_RULES.version;
    let format_line = if format.major == 0 {
        format!("0.{}", format.minor)
    } else {
        format.major.to_string()
    };
    format!("{major}.{minor}-format-{format_line}")
}

/// The files of a version of the Clean-up rules from `source`, published
/// at `published_at` and expiring at `expires_at`, whose EasyList holds
/// `ads` and EasyPrivacy `trackers`, and its digest by the Browser cache's
/// tree rule.
fn version(
    source: &str,
    published_at: jiff::Timestamp,
    expires_at: jiff::Timestamp,
    ads: &str,
    trackers: &str,
) -> (String, Vec<(&'static str, Vec<u8>)>) {
    let manifest = json!({
        "format_version": CLEAN_UP_RULES.version.to_string(),
        "source": source,
        "published_at": published_at,
        "expires_at": expires_at,
        "lists": {
            "ads": "easylist.txt",
            "trackers": "easyprivacy.txt",
            "annoyances": "fanboy-annoyance.txt",
            "unbreak": "brave-unbreak.txt",
        },
    });
    let files = vec![
        ("manifest.json", format!("{manifest:#}\n").into_bytes()),
        (
            "easylist.txt",
            format!("[Adblock Plus 2.0]\n{ads}").into_bytes(),
        ),
        (
            "easyprivacy.txt",
            format!("[Adblock Plus 2.0]\n{trackers}").into_bytes(),
        ),
        ("fanboy-annoyance.txt", b"[Adblock Plus 2.0]\n".to_vec()),
        ("brave-unbreak.txt", b"[Adblock Plus 2.0]\n".to_vec()),
    ];
    let folder = tempfile::tempdir().unwrap();
    for (name, bytes) in &files {
        std::fs::write(folder.path().join(name), bytes).unwrap();
    }
    let digest = goodfetch_engine::browser_cache::content_digest(folder.path())
        .unwrap()
        .to_string();
    (digest, files)
}

/// `days` days from now, or before it when negative.
fn days_from_now(days: i64) -> jiff::Timestamp {
    jiff::Timestamp::now()
        .checked_add(jiff::SignedDuration::from_hours(24 * days))
        .unwrap()
}

/// Writes `env`'s user configuration so that the stand-in's authority is
/// trusted for its hosts.
fn trust(env: &TestEnv, stand_in: &StandIn) {
    let authority = env.folder().join("stand-in authority.pem");
    stand_in.write_authority(&authority);
    let file = format!("'{}'", authority.display());
    env.user_configuration(&format!(
        "[settings.tls.trusted_cas]\n\"{REGISTRY}\" = [{file}]\n\"{LISTS}\" = [{file}]\n"
    ));
}

/// The Run settings that map the stand-in's hosts, and the Proving
/// ground's when there is one, to loopback, beside `clean_up`.
fn settings(proving_ground: Option<&ProvingGround>, clean_up: Value) -> Map<String, Value> {
    let mut resolve: Map<String, Value> = proving_ground
        .map(|proving_ground| {
            proving_ground
                .description()
                .resolve
                .iter()
                .map(|(host, address)| (host.clone(), json!([address])))
                .collect()
        })
        .unwrap_or_default();
    for host in [REGISTRY, LISTS] {
        resolve.insert(host.to_owned(), json!(["127.0.0.1"]));
    }
    let mut settings = Map::new();
    settings.insert("network".to_owned(), json!({ "resolve": resolve }));
    settings.insert("clean_up".to_owned(), clean_up);
    settings
}

/// Starts an Attached engine on `env` whose Browser cache is `cache`.
fn engine_with_browser<'env>(env: &'env TestEnv, cache: &Path) -> Attached<'env> {
    let mut engine = Attached::start_waiting(env, BROWSER_DEADLINE, |command| {
        command.env(goodfetch_test_support::BROWSERS_PATH_VARIABLE, cache);
    });
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    engine
}

/// Answers `answer`'s result, failing the test with it when it has none.
fn result(answer: Value) -> Value {
    match answer {
        Value::Object(mut fields) if fields.contains_key("result") => fields["result"].take(),
        answer => panic!("{answer}"),
    }
}

/// Opens a page of the Run `run_id`, loads `clean-up.test`'s `/ads` page
/// on it, and answers which of its ad and tracker the Proving ground
/// received a request for during the load.
fn loads(
    engine: &mut Attached,
    id: i64,
    proving_ground: &ProvingGround,
    run_id: &str,
) -> (bool, bool) {
    let page: PageOpened = opened(&call(
        engine,
        id,
        "run.new_page",
        &json!({ "run_id": run_id }),
    ));
    let before = requests(proving_ground);
    let url = proving_ground.url("clean-up.test", "/ads");
    let loaded = call(
        engine,
        id + 1,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": url }),
    );
    assert_eq!(
        loaded["result"]["response"]["status"],
        json!(200),
        "{loaded}"
    );
    let after = requests(proving_ground);
    let during: Vec<&String> = after.iter().skip(before.len()).collect();
    (
        during.iter().any(|target| *target == AD),
        during.iter().any(|target| *target == TRACKER),
    )
}

/// The targets of the requests `clean-up.test` received, in order.
fn requests(proving_ground: &ProvingGround) -> Vec<String> {
    proving_ground.observations().unwrap()["requests"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|request| request["host"] == json!("clean-up.test"))
        .filter_map(|request| request["target"].as_str().map(str::to_owned))
        .collect()
}

/// The sources and digests of the Clean-up rules Provenance entries of
/// `run_id` in `home`, with whether each was a resume.
fn held(home: &Path, run_id: &str) -> Vec<(Value, Value, Value)> {
    rules_provenance(home, run_id)
        .into_iter()
        .map(|entry| {
            (
                entry["source"].clone(),
                entry["digest"].clone(),
                entry["resumed"].clone(),
            )
        })
        .collect()
}

/// Two Runs started on different days' daily versions, in two homes, each
/// hold to their own, and a Run resumed from its Checkpoint in the other
/// home fetches its version by digest and blocks exactly what it blocked
/// before, by the Proving ground's record of the requests it received.
/// Covers: story:5-97, setting:clean_up.update, setting:clean_up.channel, operation:run.resume, format:clean-up-rules
#[cfg(unix)]
#[test]
fn runs_on_two_days_daily_versions_keep_their_own_and_a_resume_in_another_home_blocks_the_same() {
    let (first_home, second_home) = (TestEnv::new(), TestEnv::new());
    let proving_ground = ProvingGround::start(&first_home, SEED).unwrap();
    let stand_in = StandIn::start(&[REGISTRY, LISTS]);
    trust(&first_home, &stand_in);
    trust(&second_home, &stand_in);
    let daily = json!({ "update": "daily", "channel": channel(&stand_in.origin(REGISTRY)) });
    let run_settings = settings(Some(&proving_ground), daily);
    let (first_day, first_files) = version(
        "daily",
        days_from_now(-1),
        days_from_now(13),
        &format!("{AD}\n"),
        "",
    );
    let (second_day, second_files) = version(
        "daily",
        days_from_now(0),
        days_from_now(14),
        "",
        &format!("{TRACKER}\n"),
    );

    stand_in.publish(&channel_tag(), &first_day, &first_files);
    let first_cache = browser_cache(&first_home).unwrap();
    let mut first = engine_with_browser(&first_home, &first_cache);
    result(call(
        &mut first,
        1,
        "run.start",
        &json!({ "run_id": "first", "settings": run_settings }),
    ));
    let first_before = loads(&mut first, 2, &proving_ground, "first");
    result(call(
        &mut first,
        4,
        "run.pause",
        &json!({ "run_id": "first" }),
    ));
    let _ = first.finish();
    let checkpoint = first_home
        .home()
        .join("0.0")
        .join("runs")
        .join("first")
        .join("checkpoint.sqlite");

    stand_in.publish(&channel_tag(), &second_day, &second_files);
    let second_cache = browser_cache(&second_home).unwrap();
    let mut second = engine_with_browser(&second_home, &second_cache);
    result(call(
        &mut second,
        1,
        "run.start",
        &json!({ "run_id": "second", "settings": run_settings }),
    ));
    let second_loads = loads(&mut second, 2, &proving_ground, "second");
    result(call(
        &mut second,
        4,
        "run.resume",
        &json!({ "run_id": "first", "checkpoint": checkpoint, "settings": run_settings }),
    ));
    let first_after = loads(&mut second, 5, &proving_ground, "first");
    result(call(
        &mut second,
        7,
        "run.finish",
        &json!({ "run_id": "first" }),
    ));
    result(call(
        &mut second,
        8,
        "run.finish",
        &json!({ "run_id": "second" }),
    ));
    let _ = second.finish();

    assert_eq!(
        (
            (first_before, first_after, second_loads),
            held(first_home.home(), "first"),
            held(second_home.home(), "first"),
            held(second_home.home(), "second"),
            (
                recorded_digest(second_home.home(), "first"),
                recorded_digest(second_home.home(), "second"),
            ),
            stand_in
                .asked()
                .contains(&manifest_path(&first_day.replacen(':', "-", 1))),
        ),
        (
            ((false, true), (false, true), (true, false)),
            vec![(json!("daily"), json!(first_day), json!(false))],
            vec![(json!("daily"), json!(first_day), json!(true))],
            vec![(json!("daily"), json!(second_day), json!(false))],
            (Some(first_day.clone()), Some(second_day.clone())),
            true,
        )
    );
}

/// `clean_up.update = upstream` fetches the Filter lists from the URLs
/// `clean_up.lists` names, drops their cookie sections, and the Run holds
/// to the version they make, by its digest, named in Provenance.
/// Covers: story:5-97, setting:clean_up.update, setting:clean_up.lists, format:clean-up-rules
#[cfg(unix)]
#[test]
fn upstream_lists_are_fetched_from_their_urls_without_their_cookie_sections() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let stand_in = StandIn::start(&[REGISTRY, LISTS]);
    trust(&env, &stand_in);
    stand_in.serve(
        "/easylist.txt",
        format!(
            "[Adblock Plus 2.0]\n! Expires: 4 days\n{AD}\n! *** easylist:easylist_cookie/cookie_general_block.txt ***\n{TRACKER}\n"
        )
        .into_bytes(),
    );
    for list in [
        "/easyprivacy.txt",
        "/fanboy-annoyance.txt",
        "/brave-unbreak.txt",
    ] {
        stand_in.serve(list, b"[Adblock Plus 2.0]\n! Expires: 4 days\n".to_vec());
    }
    let origin = stand_in.origin(LISTS);
    let upstream = json!({
        "update": "upstream",
        "lists": {
            "ads": format!("https://{origin}/easylist.txt"),
            "trackers": format!("https://{origin}/easyprivacy.txt"),
            "annoyances": format!("https://{origin}/fanboy-annoyance.txt"),
            "unbreak": format!("https://{origin}/brave-unbreak.txt"),
        },
    });
    let cache = browser_cache(&env).unwrap();
    let mut engine = engine_with_browser(&env, &cache);

    result(call(
        &mut engine,
        1,
        "run.start",
        &json!({ "run_id": "upstream", "settings": settings(Some(&proving_ground), upstream) }),
    ));
    let loaded = loads(&mut engine, 2, &proving_ground, "upstream");
    result(call(
        &mut engine,
        4,
        "run.finish",
        &json!({ "run_id": "upstream" }),
    ));
    let _ = engine.finish();

    let recorded = recorded_digest(env.home(), "upstream");
    assert_eq!(
        (
            loaded,
            held(env.home(), "upstream"),
            recorded.as_deref() == Some(EMBEDDED),
        ),
        (
            (false, true),
            vec![(json!("upstream"), json!(recorded), json!(false))],
            false,
        )
    );
}

/// A Run that `clean_up.version` pins to a daily version older than 14
/// days, and past its expiry, holds to it, and its Run report warns that
/// its rules are stale.
/// Covers: story:5-97, setting:clean_up.version, setting:clean_up.channel, format:clean-up-rules
#[test]
fn a_run_pinned_to_a_version_older_than_14_days_holds_to_it_with_a_stale_warning() {
    let env = TestEnv::new();
    let stand_in = StandIn::start(&[REGISTRY, LISTS]);
    trust(&env, &stand_in);
    let (old, files) = version("daily", days_from_now(-20), days_from_now(-6), "", "");
    stand_in.publish("an-old-tag", &old, &files);
    let pinned = json!({ "version": old, "channel": channel(&stand_in.origin(REGISTRY)) });

    let report = start_and_finish(&env, "pinned", &Value::Object(settings(None, pinned)));

    let warnings = warning_codes(&report);
    assert_eq!(
        (
            recorded_digest(env.home(), "pinned"),
            held(env.home(), "pinned"),
            warnings.contains(&"clean_up_rules_stale".to_owned()),
            warnings.contains(&"clean_up_rules_fallback".to_owned()),
        ),
        (
            Some(old.clone()),
            vec![(json!("daily"), json!(old), json!(false))],
            true,
            false,
        )
    );
}

/// When the daily channel's newest version is past its expiry, a new Run
/// holds to the embedded part instead, and its Run report and Provenance
/// say so.
/// Covers: story:5-97, setting:clean_up.update, setting:clean_up.channel
#[test]
fn an_expired_daily_version_falls_back_to_the_embedded_part_with_a_warning() {
    let env = TestEnv::new();
    let stand_in = StandIn::start(&[REGISTRY, LISTS]);
    trust(&env, &stand_in);
    let (expired, files) = version("daily", days_from_now(-15), days_from_now(-1), "", "");
    stand_in.publish(&channel_tag(), &expired, &files);
    let daily = json!({ "update": "daily", "channel": channel(&stand_in.origin(REGISTRY)) });

    let report = start_and_finish(&env, "expired", &Value::Object(settings(None, daily)));

    let entries = rules_provenance(env.home(), "expired");
    assert_eq!(
        (
            recorded_digest(env.home(), "expired"),
            held(env.home(), "expired"),
            warning_codes(&report).contains(&"clean_up_rules_fallback".to_owned()),
            entries
                .first()
                .and_then(|entry| entry["fallback"].as_str())
                .is_some_and(|why| why.contains("past its expiry")),
        ),
        (
            Some(EMBEDDED.to_owned()),
            vec![(json!("embedded"), json!(EMBEDDED), json!(false))],
            true,
            true,
        )
    );
}
