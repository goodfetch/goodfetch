//! The limits on what the HTTP tiers read (ADR 0067, SC-05, pitfall H14),
//! on reqwest and `wreq` under the fetch driver, against the Proving
//! ground: every page of the hostile-input limits table that serves a body
//! ends in `response_limit_exceeded` naming its setting, the decompression
//! bombs (stacked codings included), the bodies past 64 MiB (a gzip one
//! within the ratio included) and the endless bodies, while the driver
//! stops reading at the limit; a normal compressed page passes. The driver
//! parses no JSON, so the table's JSON depth case is read as a crawl's site
//! preferences read it: the host's `TDMRep` file, fetched through the driver.
//!
//! Covers: row:SC-05, error:response_limit_exceeded
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::response_limits::Limits;
use goodfetch_engine::robots_txt::Answer;
use goodfetch_engine::settings::{Layers, Values, resolve};
use goodfetch_fetch_tier::{
    FetchBounds, Fetched, Hop, HttpProfile, NotAllowed, Request, Reqwest, SitePreferences,
    Transport, Wreq, fetch,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_FIRST_BYTE, TIMEOUT_NAVIGATION};
use goodfetch_test_support::{
    ExpectedError, HostileTables, LimitSetting, ProvingGround, ReceivedRequests, ServedBy, TestEnv,
};
use rstest::rstest;
use serde_json::{Value, json};
use url::Url;

use crate::{Stack, running_clock_runtime};

const SEED: u64 = 644;

/// The hosts of the table's pages, reached over TLS.
const HOSTS: [&str; 2] = ["bombs.test", "body-limits.test"];

/// `stack`'s transport, trusting the Proving ground's authority for the
/// table's hosts.
fn transport(stack: Stack, proving_ground: &ProvingGround) -> Box<dyn Transport> {
    let config = crate::config(proving_ground, None, &HOSTS);
    match stack {
        Stack::Reqwest => Box::new(Reqwest::new(config).unwrap()),
        Stack::Wreq => Box::new(Wreq::new(config, HttpProfile::default()).unwrap()),
    }
}

/// `timeout.first_byte` and `timeout.navigation` at 60 s each, so only a
/// limit stops an endless body, read under `limits`.
fn bounds(limits: Limits) -> FetchBounds {
    FetchBounds {
        first_byte: Deadline::given(&TIMEOUT_FIRST_BYTE, 60_000, SettingsLayer::Run),
        navigation: Deadline::given(&TIMEOUT_NAVIGATION, 60_000, SettingsLayer::Run),
        limits,
    }
}

/// Fetches the start page of the task `id` through `transport` under
/// `limits`.
fn fetched(
    transport: &dyn Transport,
    proving_ground: &ProvingGround,
    id: &str,
    limits: Limits,
) -> Result<Fetched, CallError> {
    let hop = Hop::get(Url::parse(&proving_ground.start_url(id).unwrap()).unwrap());
    running_clock_runtime().block_on(fetch(transport, &hop, &bounds(limits), &Clock::system()))
}

/// The setting and limit a `response_limit_exceeded` names, or what the
/// fetch gave instead.
fn refusal(fetched: &Result<Fetched, CallError>) -> String {
    match fetched {
        Err(error) if error.code() == "response_limit_exceeded" => {
            let details = error.details().unwrap();
            format!(
                "{}={}",
                details["setting"].as_str().unwrap(),
                details["limit"]
            )
        }
        Err(error) => format!("{}: {}", error.code(), error.message()),
        Ok(fetched) => format!(
            "Ok: status {}, {} bytes",
            fetched.status,
            fetched.body.len()
        ),
    }
}

/// What a crawl's site preferences under `max_json_depth` say of the start
/// page of the task `id`, its `TDMRep` file fetched through `transport`: the
/// setting and limit of the refusal, or `allowed`.
fn judged(
    transport: &dyn Transport,
    proving_ground: &ProvingGround,
    id: &str,
    max_json_depth: u64,
) -> String {
    let Value::Object(given) = json!({
        "network": { "max_json_depth": max_json_depth },
        "site_preferences": {
            "respect_robots_txt": false,
            "respect_use_signals": true,
            "purpose": "ai-input",
        },
    }) else {
        unreachable!("a JSON object");
    };
    let layers = Layers {
        run: Values::read(SettingsLayer::Run, &given, Product::Foxhound).unwrap(),
        ..Layers::default()
    };
    let settings = resolve(&layers, Product::Foxhound, Product::Foxhound).unwrap();
    let preferences = SitePreferences::of(&settings);
    let url = Url::parse(&proving_ground.start_url(id).unwrap()).unwrap();
    let judged = running_clock_runtime().block_on(preferences.judged(
        Request::CrawlRequest,
        &url,
        None,
        |location| async move {
            match fetch(
                transport,
                &Hop::get(location),
                &bounds(Limits::default()),
                &Clock::system(),
            )
            .await
            {
                Ok(fetched) => Answer::Response {
                    status: fetched.status,
                    body: fetched.body.to_vec(),
                },
                Err(error) => Answer::Unreachable(error.message().to_owned()),
            }
        },
    ));
    match judged {
        Ok(_) => "allowed".to_owned(),
        Err(NotAllowed::PastALimit(limit)) => refusal(&Err(limit.error())),
        Err(NotAllowed::Refused(refused)) => format!("refused: {refused}"),
    }
}

/// The driver's bytes of the host's JSON reach the one check on each
/// transport: JSON 129 deep is refused at the default and read under a depth
/// raised past it, and JSON 128 deep, which the parser alone would refuse,
/// passes the default.
///
/// Covers: setting:network.max_json_depth
#[rstest]
#[case::past_the_default("body-limits/4", 128, "network.max_json_depth=128")]
#[case::under_a_raised_depth("body-limits/4", 129, "allowed")]
#[case::at_the_default("body-limits/5", 128, "allowed")]
#[case::under_a_lowered_depth("body-limits/5", 127, "network.max_json_depth=127")]
fn the_runs_json_depth_decides_on_each_transport(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
    #[case] task: &str,
    #[case] depth: u64,
    #[case] wanted: &str,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground);

    let got = judged(transport.as_ref(), &proving_ground, task, depth);

    assert_eq!(got, wanted);
}

/// Every case of the committed limits table that a Proving ground page
/// serves and that names a setting, read on each transport: each is refused
/// naming its setting and its value, every miss collected before the test
/// fails. The cases come from the table alone, so a case it gains runs here
/// too; the test only checks that each setting is among them.
///
/// Covers: setting:network.max_body_size, setting:network.max_decompression_ratio, setting:network.max_json_depth
#[rstest]
fn every_body_of_the_limits_table_ends_in_response_limit_exceeded_naming_its_setting(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground);
    let tables = HostileTables::committed().unwrap();
    let cases: Vec<(&str, &str, LimitSetting, &str)> = tables
        .limits
        .cases
        .iter()
        .filter_map(|case| match (&case.served_by, &case.expected) {
            (ServedBy::Page(task), ExpectedError::ResponseLimitExceeded { setting }) => {
                let wanted = match setting {
                    LimitSetting::MaxBodySize => "network.max_body_size=67108864",
                    LimitSetting::MaxDecompressionRatio => "network.max_decompression_ratio=100",
                    LimitSetting::MaxJsonDepth => "network.max_json_depth=128",
                };
                Some((case.name.as_str(), task.as_str(), *setting, wanted))
            }
            _ => None,
        })
        .collect();

    let misses: Vec<String> = cases
        .iter()
        .filter_map(|(name, task, setting, wanted)| {
            let got = if *setting == LimitSetting::MaxJsonDepth {
                judged(transport.as_ref(), &proving_ground, task, 128)
            } else {
                refusal(&fetched(
                    transport.as_ref(),
                    &proving_ground,
                    task,
                    Limits::default(),
                ))
            };
            (got != *wanted).then(|| format!("{name} ({task}): wanted {wanted}, got {got}"))
        })
        .collect();

    for setting in [
        LimitSetting::MaxBodySize,
        LimitSetting::MaxDecompressionRatio,
        LimitSetting::MaxJsonDepth,
    ] {
        assert!(
            cases.iter().any(|(_, _, served, _)| *served == setting),
            "the table serves no case naming {setting:?}"
        );
    }
    assert_eq!(misses, Vec::<String>::new());
}

/// The positive control: a page compressed well within the ratio is read
/// whole on each transport.
#[rstest]
fn a_normal_compressed_page_is_read(#[values(Stack::Reqwest, Stack::Wreq)] stack: Stack) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground);

    let page = fetched(
        transport.as_ref(),
        &proving_ground,
        "bombs/6",
        Limits::default(),
    )
    .unwrap();

    let page = String::from_utf8(page.body.to_vec()).unwrap();
    assert!(page.contains("<article class=\"record\""), "{page}");
}

/// A Run that raises `network.max_decompression_ratio` reads a bomb the
/// default refuses whole on each transport: the ratio is the Run's, not a
/// fixed cap.
///
/// Covers: setting:network.max_decompression_ratio
#[rstest]
fn a_bomb_is_read_whole_under_a_raised_ratio(#[values(Stack::Reqwest, Stack::Wreq)] stack: Stack) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground);
    let raised = Limits {
        max_decompression_ratio: 100_000,
        ..Limits::default()
    };

    let bomb = fetched(transport.as_ref(), &proving_ground, "bombs/1", raised)
        .map(|fetched| fetched.body.len());

    assert_eq!(bomb.ok(), Some(10 * 1024 * 1024));
}

/// The bytes the Proving ground sent for the requests to `host`, as its
/// connection record has them once each answer ended.
fn sent_to(proving_ground: &ProvingGround, host: &str) -> Vec<u64> {
    let observations = proving_ground.observations().unwrap();
    ReceivedRequests::of(&observations)
        .every()
        .filter(|request| request.host.starts_with(host))
        .filter_map(|request| {
            request
                .delivery
                .as_ref()
                .and_then(|delivery| delivery["body_sent"].as_u64())
        })
        .collect()
}

/// An endless body is stopped at the size the Run sets: the driver stops
/// reading there, so the Proving ground sends little more than it.
///
/// Covers: setting:network.max_body_size
#[rstest]
fn an_endless_body_is_stopped_at_the_size_the_run_sets(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground);
    let limits = Limits {
        max_body_size: 1024 * 1024,
        ..Limits::default()
    };

    let endless = proving_ground.start_url("body-limits/2").unwrap();
    let host = Url::parse(&endless).unwrap().host_str().unwrap().to_owned();

    let refused = fetched(transport.as_ref(), &proving_ground, "body-limits/2", limits);
    drop(transport);
    let sent = (0..200)
        .find_map(|_| {
            let sent = sent_to(&proving_ground, &host).first().copied();
            if sent.is_none() {
                #[expect(
                    clippy::disallowed_methods,
                    reason = "the test waits for the Proving ground's record of how the answer ended"
                )]
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            sent
        })
        .expect("the Proving ground recorded the endless body's delivery");

    assert_eq!(refusal(&refused), "network.max_body_size=1048576");
    assert!(
        sent < 16 * 1024 * 1024,
        "the driver stopped reading near the limit, the Proving ground sent {sent} bytes"
    );
}
