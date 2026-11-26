//! A Secret placeholder in a `page.request` header value or JSON body
//! string (spec #564, ticket #656; ADRs 0067 and 0092, H21), against the
//! Proving ground's `canaries/4` endpoint, whose record of every request
//! that carried the planted header value canary is the oracle: the value
//! reaches the bound domain, at every hop a redirect inside the binding
//! makes too, and nowhere else, and is in no reply, event, stderr line or
//! file of the test's folder. Each case of the `secret_redirects`
//! hostile-input table fails with `secret_domain_not_allowed` and its
//! second host receives nothing, beside the paired positive control: the
//! same redirect without a placeholder reaches it. A header value holding
//! a CR, an LF or a NUL is refused with `header_value_invalid` before
//! anything is sent, beside the legitimate value's.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::Path;

use goodfetch_test_support::{
    Canaries, HostileTables, Output, Placement, ProvingGround, SecretKind, SecretPlacement,
    TestEnv, browser_cache, chrome_for_testing_files,
};
use serde_json::{Map, Value, json};
use url::Url;

use crate::canaries::paths_but;
use crate::page::{opened, started_with};
use goodfetch_test_client::Attached;

/// The Proving ground's own seed for these tests.
const SEED: u64 = 656;

/// The engine's variable the binding reads the planted value from.
const VARIABLE: &str = "GF_TEST_HEADER_VALUE";

/// The Secret placeholder the requests carry.
const PLACEHOLDER: &str = "api_token";

/// The binding's domain, the canaries family's.
const BOUND: &str = "canaries.test";

/// The endpoint's host, under the binding.
const ENDPOINT_HOST: &str = "4.canaries.test";

/// The endpoint that takes the value, as a Bearer token or the JSON
/// body's `token`.
const RECORD: &str = "/api/record";

/// The names outside the binding the redirects go to, which the Run maps
/// to the Proving ground.
const ELSEWHERE: [&str; 2] = ["elsewhere.test", "canaries.test.elsewhere.test"];

/// The Run's settings: [`PLACEHOLDER`] bound to [`VARIABLE`] for
/// [`BOUND`], and [`ELSEWHERE`]'s names mapped to the Proving ground.
fn settings(proving_ground: &ProvingGround) -> Map<String, Value> {
    let address = proving_ground.address().ip().to_string();
    let resolve: Map<String, Value> = ELSEWHERE
        .iter()
        .map(|host| ((*host).to_owned(), json!([address])))
        .collect();
    let placeholders: Map<String, Value> = [(
        PLACEHOLDER.to_owned(),
        json!({ "reference": format!("env:{VARIABLE}"), "domains": [BOUND] }),
    )]
    .into_iter()
    .collect();
    [
        (
            "secrets".to_owned(),
            json!({ "placeholders": placeholders }),
        ),
        ("network".to_owned(), json!({ "resolve": resolve })),
    ]
    .into_iter()
    .collect()
}

/// The value of a canary of `kind` planted through `proving_ground` in
/// `canaries`.
fn planted(canaries: &mut Canaries, proving_ground: &ProvingGround, kind: SecretKind) -> String {
    canaries
        .plant_through(proving_ground, kind)
        .unwrap()
        .value()
        .to_owned()
}

/// An engine whose Run `run_id` binds `value`, and a page of it.
fn engine_with_page(
    env: &TestEnv,
    cache: &Path,
    proving_ground: &ProvingGround,
    value: &str,
    run_id: &str,
) -> (Attached, String) {
    let mut engine = started_with(
        env,
        cache,
        proving_ground,
        run_id,
        &settings(proving_ground),
        |command| {
            command.env(VARIABLE, value);
        },
    );
    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": run_id }))).page_id;
    (engine, page.0)
}

/// `url` as the Proving ground serves it: over HTTP on its port, its host
/// and userinfo kept.
fn served(proving_ground: &ProvingGround, url: &str) -> Url {
    let mut url = Url::parse(url).unwrap();
    url.set_scheme("http").unwrap();
    url.set_port(Some(proving_ground.address().port())).unwrap();
    url
}

/// The redirect page's URL on the endpoint's host, answering `status` to
/// `location`, as the Proving ground serves both.
fn redirect(proving_ground: &ProvingGround, status: u16, location: &str) -> String {
    let mut url = served(
        proving_ground,
        &format!("https://{ENDPOINT_HOST}/api/redirect/{status}"),
    );
    url.query_pairs_mut()
        .append_pair("to", served(proving_ground, location).as_str());
    url.into()
}

/// `page.request`'s parameters for `url`, the value at `placement`: in an
/// `Authorization` Bearer header built from the placeholder, or in the
/// JSON body's `token` the placeholder sets; without `secret`, the same
/// request with a plain value in its place.
fn request(page: &str, url: &str, placement: SecretPlacement, secret: bool) -> Value {
    match (placement, secret) {
        (SecretPlacement::Header, true) => json!({
            "page_id": page,
            "url": url,
            "secret_headers": {
                "Authorization": { "form": "value", "placeholder": PLACEHOLDER, "prefix": "Bearer " },
            },
        }),
        (SecretPlacement::Header, false) => json!({
            "page_id": page,
            "url": url,
            "headers": { "Authorization": "Bearer plain" },
        }),
        (SecretPlacement::JsonBody, secret) => {
            let mut params = json!({
                "page_id": page,
                "url": url,
                "method": "POST",
                "headers": { "Content-Type": "application/json" },
                "post_data": data_encoding::BASE64.encode(br#"{"token":"plain"}"#),
            });
            if secret {
                params["json_secrets"] =
                    json!([{ "pointer": "/token", "placeholder": PLACEHOLDER }]);
            }
            params
        }
    }
}

/// How many requests for [`RECORD`] on `host` the Proving ground received.
fn records_on(proving_ground: &ProvingGround, host: &str) -> usize {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.host == host && request.target == RECORD)
        .count()
}

/// Every request the Proving ground saw carrying a canary of `canaries`,
/// as its placement, part, host and target.
fn sightings(canaries: &Canaries, proving_ground: &ProvingGround) -> Vec<String> {
    canaries
        .sightings(proving_ground)
        .unwrap()
        .into_iter()
        .map(|sighting| {
            format!(
                "{:?} {} {} {}",
                sighting.placement, sighting.part, sighting.host, sighting.target
            )
        })
        .collect()
}

/// A header value and a JSON body string holding the placeholder reach the
/// endpoint on the bound domain, which takes the value, directly and
/// through a 307 and a 308 inside the binding, whose second hop the engine
/// fills again; the Proving ground sees the value only where it expects
/// it, and it is in no reply or event, nor the engine's stderr, nor a file
/// of the test's folder, the browser's profile included, raw or encoded.
///
/// Covers: story:4-53, operation:page.request, setting:secrets.placeholders
#[test]
fn a_placeholder_in_a_header_or_a_json_string_reaches_its_bound_domain_alone() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut canaries = Canaries::new(env.seed());
    let value = planted(&mut canaries, &proving_ground, SecretKind::HeaderValue);
    let (mut engine, page) = engine_with_page(&env, &cache, &proving_ground, &value, "bound");
    let direct = proving_ground.url(ENDPOINT_HOST, RECORD);
    let inside = format!("https://{ENDPOINT_HOST}{RECORD}");

    let mut replies: Vec<Value> = [
        (direct.clone(), SecretPlacement::Header),
        (direct, SecretPlacement::JsonBody),
        (
            redirect(&proving_ground, 307, &inside),
            SecretPlacement::Header,
        ),
        (
            redirect(&proving_ground, 308, &inside),
            SecretPlacement::JsonBody,
        ),
    ]
    .into_iter()
    .zip(3..)
    .map(|((url, placement), id)| {
        engine.ask(id, "page.request", &request(&page, &url, placement, true))
    })
    .collect();
    replies.push(engine.ask(7, "run.finish", &json!({ "run_id": "bound" })));
    let (rest, stderr) = engine.finish_with_stderr();
    replies.extend(rest);

    let replies_bytes = serde_json::to_vec(&replies).unwrap();
    let searched = paths_but(env.folder(), &chrome_for_testing_files(&cache));
    let mut outputs = vec![
        Output::Bytes {
            name: "replies",
            bytes: &replies_bytes,
        },
        Output::Bytes {
            name: "stderr",
            bytes: stderr.as_bytes(),
        },
    ];
    outputs.extend(searched.iter().map(|path| Output::Path(path)));
    let leaks: Vec<String> = canaries
        .leaks(&outputs)
        .unwrap()
        .into_iter()
        .map(|hit| format!("{:?} in {}", hit.encoding, hit.output))
        .collect();
    let sightings = sightings(&canaries, &proving_ground);
    let planted = format!("{:?}", Placement::Planted);
    assert_eq!(
        (
            replies[..4]
                .iter()
                .map(|reply| reply["result"]["status"].clone())
                .collect::<Vec<_>>(),
            sightings
                .iter()
                .filter(|sighting| sighting.starts_with(&planted))
                .count(),
            leaks,
        ),
        (vec![json!(200); 4], sightings.len(), Vec::<String>::new()),
        "{sightings:?}\n{replies:?}\n{stderr}"
    );
    assert!(
        sightings
            .iter()
            .any(|sighting| sighting.contains("header:authorization"))
            && sightings.iter().any(|sighting| sighting.contains(" body ")),
        "{sightings:?}"
    );
}

/// Each case of the `secret_redirects` hostile-input table, a request
/// carrying the placeholder's value in a header or a JSON body redirected
/// outside the binding, fails with `secret_domain_not_allowed`, and the
/// redirect's host receives no request; the same redirect without a
/// placeholder, the positive control, ends on that host. The Proving
/// ground sees the value only on the bound domain: it looks for canaries
/// in every request, on a host it serves pages on or not, so a look-alike
/// host such as `canaries.test.elsewhere.test`, whose requests it keeps no
/// record of, would still show a leak.
///
/// Covers: story:4-53, error:secret_domain_not_allowed, operation:page.request
#[test]
fn a_redirect_outside_the_binding_fails_with_secret_domain_not_allowed_and_carries_nothing() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut canaries = Canaries::new(env.seed());
    let value = planted(&mut canaries, &proving_ground, SecretKind::HeaderValue);
    let (mut engine, page) = engine_with_page(&env, &cache, &proving_ground, &value, "leaving");
    let tables = HostileTables::committed().unwrap();

    let mut misses = Vec::new();
    for (case, id) in tables.secret_redirects.cases.iter().zip((3..).step_by(2)) {
        let url = redirect(&proving_ground, case.input.status, &case.input.location);
        let host = served(&proving_ground, &case.input.location)
            .host_str()
            .unwrap()
            .to_owned();
        let before = records_on(&proving_ground, &host);
        let refused = engine.ask(
            id,
            "page.request",
            &request(&page, &url, case.input.placement, true),
        );
        let after_refusal = records_on(&proving_ground, &host);
        let control = engine.ask(
            id + 1,
            "page.request",
            &request(&page, &url, case.input.placement, false),
        );
        let ended_on = control["result"]["url"]
            .as_str()
            .and_then(|url| Url::parse(url).ok())
            .and_then(|url| url.host_str().map(str::to_owned));
        let outcome = (
            refused["error"]["data"]["code"].clone(),
            after_refusal - before,
            ended_on,
        );
        let expected = serde_json::to_value(&case.expected).unwrap()["code"].clone();
        if outcome != (expected, 0, Some(host)) {
            misses.push(format!("{}: {outcome:?} {refused} {control}", case.name));
        }
    }
    let _ = engine.ask(99, "run.finish", &json!({ "run_id": "leaving" }));
    let _ = engine.finish();
    let sightings = sightings(&canaries, &proving_ground);
    let planted = format!("{:?}", Placement::Planted);

    assert_eq!(
        (
            misses,
            sightings
                .iter()
                .filter(|sighting| !sighting.starts_with(&planted)
                    || !sighting.contains(&format!(" {ENDPOINT_HOST} ")))
                .collect::<Vec<_>>(),
        ),
        (Vec::<String>::new(), Vec::<&String>::new()),
        "{sightings:?}"
    );
}

/// A header value whose text holds a CR, an LF or a NUL, in its prefix or
/// its Basic username, is refused with `header_value_invalid` before the
/// request is sent; the legitimate value, the positive control, reaches
/// the endpoint, which takes it.
///
/// Covers: story:4-53, error:header_value_invalid, operation:page.request
#[test]
fn a_cr_an_lf_or_a_nul_in_a_header_value_is_refused_before_it_is_sent() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut canaries = Canaries::new(env.seed());
    let value = planted(&mut canaries, &proving_ground, SecretKind::HeaderValue);
    let (mut engine, page) = engine_with_page(&env, &cache, &proving_ground, &value, "headers");
    let url = proving_ground.url(ENDPOINT_HOST, RECORD);
    let header = |header: Value| json!({ "page_id": page, "url": url, "secret_headers": { "Authorization": header } });
    let value =
        |prefix: &str| json!({ "form": "value", "placeholder": PLACEHOLDER, "prefix": prefix });

    let refused: Vec<Value> = [
        value("Bearer\r\nX-Injected: yes\r\n"),
        value("Bearer\n"),
        value("Bearer\r"),
        value("Bearer\0"),
        json!({ "form": "basic", "username": "ann\r\nX-Injected: yes", "placeholder": PLACEHOLDER }),
    ]
    .into_iter()
    .zip(3..)
    .map(|(form, id)| engine.ask(id, "page.request", &header(form))["error"]["data"]["code"].clone())
    .collect();
    let sent_before = records_on(&proving_ground, ENDPOINT_HOST);
    let legitimate = engine.ask(8, "page.request", &header(value("Bearer ")));
    let _ = engine.ask(9, "run.finish", &json!({ "run_id": "headers" }));
    let _ = engine.finish();

    assert_eq!(
        (
            refused,
            sent_before,
            &legitimate["result"]["status"],
            records_on(&proving_ground, ENDPOINT_HOST),
        ),
        (vec![json!("header_value_invalid"); 5], 0, &json!(200), 1),
        "{legitimate}"
    );
}
