//! What a browser the engine launches does on its own (ADRs 0017 and 0043,
//! ticket #639): it calls no browser vendor while a program navigates and
//! idles, and it opens no port a local process could debug it through,
//! since the engine reaches it over a pipe. Each test runs Collie's
//! executable against a real Chrome for Testing and its own Proving ground.
//!
//! The vendor test's oracle is the Proving ground's connection record. Its
//! profile, which the engine creates in the Run's folder and launches on,
//! is written beforehand with Chromium's proxy preference, in `Local State`
//! for the browser's own network context (updates, accounts, metrics) and
//! in the profile's `Preferences` for its pages, so every connection the
//! browser opens is a relay of the Proving ground's open forward proxy,
//! which records the host each asks for and connects nowhere but this
//! host's loopback: nothing leaves the machine, whatever the browser asks.
//! Under ADR 0043's deny in CI, a connection that bypassed the proxy would
//! also fail the job.
#![expect(
    clippy::disallowed_methods,
    reason = "the test writes the profile, lists processes and sockets and waits on its own thread, never an async worker"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use goodfetch_engine::pinned_download::chromium_background_traffic_off;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{
    ProvingGround, TestEnv, browser_cache, chrome_headless, reported_debugging_port,
    skipped_without_gui_session,
};
use serde_json::{Value, json};

use crate::page::{opened, started, started_with};

/// The Proving ground's own seed for these tests.
const SEED: u64 = 639;

/// How long the browser idles after its last navigation: past the startup
/// calls a browser makes, each retried after a failure, and its on-demand
/// component installs.
const IDLE: Duration = Duration::from_secs(10);

/// The Browser data folder the engine launches the Run's browser on.
fn launch_data_folder(env: &TestEnv, run_id: &str) -> PathBuf {
    HomeLayout::of(env.home())
        .run_folder(run_id)
        .join("browser-data")
        .join("1")
}

/// Writes Chromium's proxy preference, naming `proxy`, into the browser's
/// state and the profile's preferences in `profile`.
fn through_proxy(profile: &Path, proxy: &str) {
    let preference = json!({
        "proxy": { "mode": "fixed_servers", "server": format!("http://{proxy}") }
    })
    .to_string();
    std::fs::create_dir_all(profile.join("Default")).unwrap();
    std::fs::write(profile.join("Local State"), &preference).unwrap();
    std::fs::write(profile.join("Default").join("Preferences"), &preference).unwrap();
}

/// The host each relay of the Proving ground's forward proxies asked for:
/// a `CONNECT` target's `host:port`, or an absolute URL's host.
fn relayed_hosts(observations: &Value) -> BTreeSet<String> {
    observations["relays"]
        .as_array()
        .unwrap()
        .iter()
        .map(|relay| {
            let target = relay["target"].as_str().unwrap();
            let authority = target
                .split_once("://")
                .map_or(target, |(_, rest)| rest)
                .split('/')
                .next()
                .unwrap();
            authority
                .rsplit_once(':')
                .map_or(authority, |(host, _)| host)
                .to_owned()
        })
        .collect()
}

/// A browser launched by the engine, its traffic routed through the Proving
/// ground's forward proxy, asks for no host but the Proving ground's while
/// a program opens two of its pages and the browser idles. The relays name
/// the pages' hosts and the `.invalid` names the background-traffic table
/// points the browser's own endpoints at: both the pages' traffic and the
/// browser's own reach the record. Its Run has no egress relay, so its
/// pages too go through the profile's proxy.
/// Covers: story:4-83
#[test]
fn a_launched_browser_calls_no_vendor_host_through_navigation_and_idle_time() {
    if skipped_without_gui_session(
        "it routes the browser through a proxy its profile's preferences name, which the headless shell does not read",
    ) {
        return;
    }
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    // Its pages take the profile's proxy, not the Session's egress relay,
    // which every page's context would otherwise take as its own.
    let mut settings = serde_json::Map::new();
    settings.insert(
        "security".to_owned(),
        json!({ "raw_cdp_egress_proxy": false }),
    );
    let mut engine = started_with(&env, &cache, &proving_ground, "quiet", &settings, |_| {});
    through_proxy(
        &launch_data_folder(&env, "quiet"),
        &proving_ground.description().fixtures.proxies["open"],
    );

    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "quiet" })));
    let first = engine.ask(3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url("catalog.test", "/catalog/1") }),
    );
    let second = engine.ask(4,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url("catalog.test", "/catalog/2") }),
    );
    std::thread::sleep(IDLE);
    let _ = engine.ask(5, "run.finish", &json!({ "run_id": "quiet" }));
    let _ = engine.finish();
    let hosts = relayed_hosts(&proving_ground.observations().unwrap());

    let served = &proving_ground.description().resolve;
    let outside: Vec<&String> = hosts
        .iter()
        .filter(|host| !served.contains_key(*host) && !host.ends_with(".invalid"))
        .collect();
    assert_eq!(
        (
            &first["result"]["response"]["status"],
            &second["result"]["response"]["status"],
            hosts.contains("catalog.test"),
            hosts.iter().any(|host| host.ends_with(".invalid")),
            outside,
        ),
        (&json!(200), &json!(200), true, true, Vec::<&String>::new()),
        "{hosts:?}"
    );
}

/// The IDs of the processes whose command line names a profile folder
/// called `profile` in the test's folder, which `ps` names by the folder
/// name's last, ASCII, part.
fn processes_on(env: &TestEnv, profile: &str) -> BTreeSet<String> {
    let name = env.folder().file_name().unwrap().to_string_lossy();
    let marker = name.rsplit(' ').next().unwrap().to_owned();
    let listed = Command::new("ps")
        .args(["-axww", "-o", "pid=,args="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter(|line| line.contains(&marker) && line.contains(profile))
        .filter_map(|line| Some(line.split_whitespace().next()?.to_owned()))
        .collect()
}

/// A socket that waits for others to connect: a listening TCP socket or an
/// unconnected UDP one, with its process ID, protocol and address.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Listening {
    process: String,
    protocol: String,
    address: String,
}

/// Every listening TCP socket and unconnected UDP socket on the host, as
/// `lsof` lists them.
fn listening_sockets() -> Vec<Listening> {
    let listed = Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-iUDP", "-FnPp"])
        .output()
        .unwrap();
    let mut sockets = Vec::new();
    let (mut process, mut protocol) = (String::new(), String::new());
    for line in String::from_utf8_lossy(&listed.stdout).lines() {
        let (field, value) = line.split_at(1);
        match field {
            "p" => value.clone_into(&mut process),
            "P" => value.clone_into(&mut protocol),
            "n" if !value.contains("->") => sockets.push(Listening {
                process: process.clone(),
                protocol: protocol.clone(),
                address: value.to_owned(),
            }),
            _ => {}
        }
    }
    sockets
}

/// While a program's page is open, a scan of the host's listening sockets
/// finds none of the engine's browser's processes; the test's own browser,
/// launched with a debugging port, is found listening on the port it
/// reports.
/// Covers: story:4-23
#[test]
fn an_engine_launched_browser_opens_no_port_while_a_debugging_port_launch_does() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started(&env, &cache, &proving_ground, "piped");
    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "piped" })));
    let went = engine.ask(3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url("catalog.test", "/catalog/1") }),
    );
    let debugging_profile = env.folder().join("debugging-port-profile");
    let mut command = env.process_command(chrome_headless(&cache));
    command
        .args(chromium_background_traffic_off())
        .args([
            "--headless",
            "--no-first-run",
            "--use-mock-keychain",
            "--password-store=basic",
            "--remote-debugging-port=0",
            "--host-resolver-rules=MAP * ~NOTFOUND",
        ])
        .arg(format!("--user-data-dir={}", debugging_profile.display()))
        .arg("about:blank")
        .stderr(Stdio::null());
    let debugging = env.spawn(&mut command).unwrap();
    let port = reported_debugging_port(&debugging_profile);

    let sockets = listening_sockets();

    let engines = processes_on(&env, "browser-data");
    let tests = processes_on(&env, "debugging-port-profile");
    debugging.interrupt().unwrap();
    let _ = debugging.child().wait();
    let _ = engine.ask(4, "run.finish", &json!({ "run_id": "piped" }));
    let _ = engine.finish();
    assert_eq!(
        (
            &went["result"]["response"]["status"],
            engines.is_empty(),
            sockets
                .iter()
                .filter(|socket| engines.contains(&socket.process))
                .collect::<Vec<_>>(),
            sockets.iter().any(|socket| tests.contains(&socket.process)
                && socket.protocol == "TCP"
                && socket.address.ends_with(&format!(":{port}"))),
        ),
        (&json!(200), false, Vec::<&Listening>::new(), true),
        "{sockets:?}"
    );
}
