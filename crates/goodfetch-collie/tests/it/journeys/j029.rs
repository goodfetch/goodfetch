//! J-029, "Run a program through a provider adapter and see one exit per
//! Session and its proxy bytes in Usage", on the `cli` surface (ADRs 0021,
//! 0063 and 0075; ticket #891): a person names an Oxylabs account in user
//! configuration, its endpoint the Proving ground's forward proxy, which
//! reads Oxylabs' sticky usernames, runs Collie's shipped executable as
//! `daemon run`, and a program of theirs, a Client speaking the protocol,
//! runs two Sessions through the `oxylabs` adapter.

use std::collections::BTreeSet;
use std::path::Path;

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_client::DaemonClient;
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, DaemonEndpoint, JourneyRun, ProvingGround, browser_cache,
};
use serde_json::{Map, Value, json};

use crate::daemon_run::{ANY_PORT, Foreground, opened};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The variable the account's password is read from.
const PASSWORD_VARIABLE: &str = "OXYLABS_PASSWORD";

/// The program's run.
const RUN_ID: &str = "through-oxylabs";

/// The run settings that map the Proving ground's hosts to its address, and
/// draw each Session's proxy from `oxylabs`.
fn settings(proving_ground: &ProvingGround) -> Value {
    let resolve: Map<String, Value> = proving_ground
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), json!([address])))
        .collect();
    json!({
        "network": { "resolve": resolve },
        "egress": { "private_ranges": resolve.keys().collect::<Vec<_>>() },
        "proxy": { "provider": "oxylabs" },
    })
}

/// The relayed requests whose target holds `marker`: each one's sticky
/// session, as the proxy read its username, and the exit it left from.
fn relayed(proving_ground: &ProvingGround, marker: &str) -> Vec<(Value, Value)> {
    proving_ground.observations().unwrap()["relays"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|relay| {
            relay["outcome"] == json!("relayed")
                && relay["target"]
                    .as_str()
                    .is_some_and(|target| target.contains(marker))
        })
        .map(|relay| (relay["sticky"].clone(), relay["exit"].clone()))
        .collect()
}

/// Covers: story:8-89, story:8-90
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a Journey variant is one test of its every step and check, in its file's order (TESTING.md, Journeys)"
)]
fn cli() {
    let mut run = JourneyRun::start("J-029", "cli", Path::new(COLLIE), ResultDocument::journey)
        .expect("J-029's cli variant starts");
    let cache = browser_cache(run.env()).expect("Playwright's Chrome for Testing is installed");
    let cache = cache.to_string_lossy().into_owned();
    let password = run
        .proving_ground()
        .expect("J-029 has a Proving ground")
        .description()
        .fixtures
        .proxy_credentials
        .password
        .clone();
    let variables: Vec<(&str, &str)> = vec![
        (BROWSERS_PATH_VARIABLE, &cache),
        (PASSWORD_VARIABLE, &password),
    ];
    let mut daemon: Option<Foreground> = None;
    let mut endpoint: Option<DaemonEndpoint> = None;
    let mut client: Option<DaemonClient> = None;
    let mut next_id = 0;
    let mut pages: Vec<(String, Value)> = Vec::new();
    let mut report = Value::Null;

    run.step(
        "Name an Oxylabs account in the user configuration: its proxy user, the Secret reference of its password, and the Proving ground's forward proxy as its endpoint",
        |run| {
            let proving_ground = run.proving_ground().expect("J-029 has a Proving ground");
            let fixtures = &proving_ground.description().fixtures;
            let configuration = format!(
                "daemon.address = \"{ANY_PORT}\"\n\n[settings.proxy.providers.oxylabs]\nusername = \"{}\"\npassword = \"env:{PASSWORD_VARIABLE}\"\nendpoint = \"http://{}\"\n",
                fixtures.proxy_credentials.username, fixtures.proxies["residential"],
            );
            run.env().user_configuration(&configuration);
        },
    );
    run.step("Run the Daemon in the foreground (`daemon run`)", |run| {
        let mut started = Foreground::start(run.env(), run.engine().to_str().unwrap(), &variables);
        endpoint = Some(started.serving());
        daemon = Some(started);
    });
    run.step(
        "Start a run on the Daemon from a program whose Sessions draw their proxy from `oxylabs`, and open two Sessions",
        |run| {
            let proving_ground = run.proving_ground().expect("J-029 has a Proving ground");
            let mut program = opened(run.env(), endpoint.as_ref().expect("the Daemon serves"));
            next_id += 1;
            let started = program.call(
                next_id,
                "run.start",
                &json!({ "run_id": RUN_ID, "settings": settings(proving_ground) }),
            );
            assert!(started.get("result").is_some(), "{started}");
            for name in ["first", "second"] {
                next_id += 1;
                let session = program.call(
                    next_id,
                    "run.new_session",
                    &json!({ "run_id": RUN_ID, "settings": {} }),
                );
                let session_id = session["result"]["session_id"].clone();
                assert!(!session_id.is_null(), "{session}");
                next_id += 1;
                let page = program.call(
                    next_id,
                    "session.new_page",
                    &json!({ "session_id": session_id }),
                );
                let page_id = page["result"]["page_id"].clone();
                assert!(!page_id.is_null(), "{page}");
                pages.push((name.to_owned(), page_id));
            }
            client = Some(program);
        },
    );
    run.step("Visit two pages in each Session", |run| {
        let proving_ground = run.proving_ground().expect("J-029 has a Proving ground");
        let program = client.as_mut().expect("the program is connected");
        for visit in ["one", "two"] {
            for (name, page_id) in &pages {
                next_id += 1;
                let url =
                    proving_ground.url("scope.test", &format!("/storage?mark={name}-{visit}"));
                let went = program.call(
                    next_id,
                    "page.goto",
                    &json!({ "page_id": page_id, "url": url }),
                );
                assert!(went.get("result").is_some(), "{went}");
            }
        }
    });
    run.step("Finish the run and read its report", |_| {
        let program = client.as_mut().expect("the program is connected");
        next_id += 1;
        let finished = program.call(next_id, "run.finish", &json!({ "run_id": RUN_ID }));
        report = finished["result"].clone();
        assert!(!report.is_null(), "{finished}");
        client = None;
        if let Some(daemon) = daemon.as_mut() {
            daemon.terminate();
        }
    });

    run.check(
        "The proxy saw each Session's requests under Oxylabs' sticky username, one sticky session and one exit per Session, each Session its own",
        |run| {
            let proving_ground = run.proving_ground().expect("J-029 has a Proving ground");
            let of = |name: &str| {
                let relays = relayed(proving_ground, &format!("mark={name}-"));
                let providers: BTreeSet<String> = relays
                    .iter()
                    .map(|(sticky, _)| sticky["provider"].to_string())
                    .collect();
                let sessions: BTreeSet<String> = relays
                    .iter()
                    .map(|(sticky, _)| sticky["session"].to_string())
                    .collect();
                let exits: BTreeSet<String> =
                    relays.iter().map(|(_, exit)| exit.to_string()).collect();
                (relays.len() >= 2, providers, sessions, exits.len())
            };
            let (first, second) = (of("first"), of("second"));
            let oxylabs = BTreeSet::from([json!("oxylabs").to_string()]);
            assert_eq!(
                (
                    (first.0, &first.1, first.2.len(), first.3),
                    (second.0, &second.1, second.2.len(), second.3),
                    first.2.is_disjoint(&second.2),
                ),
                ((true, &oxylabs, 1, 1), (true, &oxylabs, 1, 1), true),
                "{first:?} {second:?}"
            );
        },
    );
    run.check(
        "The Run report's Usage counts the bytes the proxy carried under the `residential` kind",
        |_| {
            let by_kind = report["usage"]["proxy_bytes_by_kind"].clone();
            let residential = by_kind
                .as_array()
                .into_iter()
                .flatten()
                .find(|entry| entry["kind"] == json!("residential"))
                .and_then(|entry| entry["bytes"].as_u64());
            assert!(residential.is_some_and(|bytes| bytes > 0), "{report}");
        },
    );
    run.finish()
        .expect("J-029's document is written")
        .assert_proved();
}
