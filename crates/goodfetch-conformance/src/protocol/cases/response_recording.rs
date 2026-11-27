//! The page suite's Response recording replay cases (ADR 0105, spec #580,
//! ticket #799): a Run started with `recording: { mode: "replay", path }`
//! answers navigations, subresources, `page.request`, `Vary` headers,
//! canonical JSON bodies and repeated calls in recorded order from its WARC
//! file without sending a single request to the Proving ground; refuses a
//! missing request with `response_not_recorded` naming method, URL,
//! recording and nearest recorded URLs; fails missing subresources and
//! lists every miss in the Run report; reads robots.txt from the recording
//! alone (treating a missing robots.txt as no rules); and fails start with
//! `recording_invalid` on an unreadable, non-WARC or unsupported-version
//! file.

mod determinism;
mod record;

use std::path::{Path, PathBuf};
use std::time::Duration;

use goodfetch_protocol::page::PAGE_GOTO;
use goodfetch_protocol::page::events::RESPONSE_BODY;
use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::page::request::PAGE_REQUEST;
use goodfetch_protocol::run::{RECORDING_INVALID, RESPONSE_NOT_RECORDED, RUN_FINISH, RUN_START};
use serde_json::{Value, json};

use super::fetch_tiers::cached::HOST as CACHE_HOST;
use super::navigation::{answered, call};
use super::{Checks, Site, new_page, poll, run_trace};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::EngineTarget;

/// The Proving ground host used in the recorded URLs: the Proving ground
/// itself receives zero requests during replay.
const HOST: &str = "1.access-policy.test";

/// The Provenance action of a robots.txt reading.
const ROBOTS_ACTION: &str = "robots_txt";

/// How long a Provenance reading may take to land after navigation.
const RECORDED_WITHIN: Duration = Duration::from_secs(20);

/// One Response recording case of the page suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    ReplaysWithoutNetwork,
    MissRaisesAndReports,
    RobotsTxtFromRecording,
    InvalidRecordingFailsStart,
    RecordAtomicAndReplay,
    RecordRedactsAndMasksCanaries,
    RecordCutAndReplayTruncated,
    RecordCacheAndKillResume,
    ReplayClockAndRandomness,
    LoopbackResolveBypassDuringReplay,
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-replaying-run-answers-pages-subresources-page-requests-vary-and-repeated-calls-without-reaching-the-proving-ground" => {
                Self::ReplaysWithoutNetwork
            }
            "a-missing-request-raises-response-not-recorded-naming-method-url-and-nearest-and-lists-every-miss-in-the-report" => {
                Self::MissRaisesAndReports
            }
            "a-replaying-run-reads-robots-txt-from-the-recording-and-treats-a-missing-one-as-no-rules" => {
                Self::RobotsTxtFromRecording
            }
            "an-unreadable-non-warc-or-unsupported-version-recording-fails-run-start-with-recording-invalid" => {
                Self::InvalidRecordingFailsStart
            }
            "a-record-run-replaces-its-warc-atomically-names-engine-and-run-in-warcinfo-and-replays-without-network" => {
                Self::RecordAtomicAndReplay
            }
            "a-recorded-warc-drops-credentials-masks-set-cookie-and-holds-no-secret-canary" => {
                Self::RecordRedactsAndMasksCanaries
            }
            "a-cut-response-is-recorded-with-warc-truncated-disconnect-and-replays-as-response-truncated" => {
                Self::RecordCutAndReplayTruncated
            }
            "a-record-run-reads-no-cache-entry-may-store-into-cache-and-merges-both-halves-after-kill-and-resume" => {
                Self::RecordCacheAndKillResume
            }
            "a-replayed-page-renders-recorded-date-and-seeded-randomness-identically-and-date-keeps-running" => {
                Self::ReplayClockAndRandomness
            }
            "a-host-mapped-by-resolve-to-loopback-is-reached-live-during-replay-while-other-hosts-replay" => {
                Self::LoopbackResolveBypassDuringReplay
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection to `target` against `site`.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        target: &EngineTarget,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::ReplaysWithoutNetwork => {
                replays_without_network(connection, target, site, checks)
            }
            Self::MissRaisesAndReports => miss_raises_and_reports(connection, target, site, checks),
            Self::RobotsTxtFromRecording => {
                robots_txt_from_recording(connection, target, site, checks)
            }
            Self::InvalidRecordingFailsStart => {
                invalid_recording_fails_start(connection, target, site, checks)
            }
            Self::RecordAtomicAndReplay => {
                record::record_atomic_and_replay(connection, target, site, checks)
            }
            Self::RecordRedactsAndMasksCanaries => {
                record::record_redacts_and_masks_canaries(connection, target, site, checks)
            }
            Self::RecordCutAndReplayTruncated => {
                record::record_cut_and_replay_truncated(connection, target, site, checks)
            }
            Self::RecordCacheAndKillResume => {
                record::record_cache_and_kill_resume(connection, target, site, checks)
            }
            Self::ReplayClockAndRandomness => {
                determinism::replay_clock_and_randomness(connection, target, site, checks)
            }
            Self::LoopbackResolveBypassDuringReplay => {
                determinism::loopback_resolve_bypass_during_replay(connection, target, site, checks)
            }
        }
    }
}

/// Writes `bytes` to `<GOODFETCH_HOME>/recordings/<name>` and returns its
/// path as a string.
#[expect(
    clippy::disallowed_methods,
    reason = "the conformance harness is synchronous and runs no async worker"
)]
fn write_fixture(target: &EngineTarget, name: &str, bytes: &[u8]) -> PathBuf {
    let base = target
        .environment
        .get("GOODFETCH_HOME")
        .map_or_else(std::env::temp_dir, PathBuf::from);
    let dir = base.join("recordings");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("fixture recording is writable");
    path
}

/// Builds a minimal WARC 1.1 byte buffer.
pub(super) struct WarcBuilder {
    bytes: Vec<u8>,
    next_id: u64,
}

impl WarcBuilder {
    pub(super) fn new(software: &str) -> Self {
        let mut builder = Self {
            bytes: Vec::new(),
            next_id: 1,
        };
        let block = format!("software: {software}\r\n");
        let id = builder.id();
        builder.push_record(
            "warcinfo",
            &id,
            Some("2026-01-05T09:00:00Z"),
            &[],
            "application/warc-fields",
            block.as_bytes(),
        );
        builder
    }

    fn id(&mut self) -> String {
        let n = self.next_id;
        self.next_id += 1;
        format!("<urn:uuid:00000000-0000-0000-0000-{n:012}>")
    }

    fn push_record(
        &mut self,
        warc_type: &str,
        record_id: &str,
        date: Option<&str>,
        extra: &[(&str, &str)],
        content_type: &str,
        block: &[u8],
    ) {
        self.bytes.extend_from_slice(b"WARC/1.1\r\n");
        self.bytes
            .extend_from_slice(format!("WARC-Type: {warc_type}\r\n").as_bytes());
        self.bytes
            .extend_from_slice(format!("WARC-Record-ID: {record_id}\r\n").as_bytes());
        if let Some(date) = date {
            self.bytes
                .extend_from_slice(format!("WARC-Date: {date}\r\n").as_bytes());
        }
        for (name, value) in extra {
            self.bytes
                .extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        self.bytes
            .extend_from_slice(format!("Content-Type: {content_type}\r\n").as_bytes());
        self.bytes
            .extend_from_slice(format!("Content-Length: {}\r\n\r\n", block.len()).as_bytes());
        self.bytes.extend_from_slice(block);
        self.bytes.extend_from_slice(b"\r\n\r\n");
    }

    /// Appends a linked `request` + `response` pair for `method` `url`.
    #[expect(
        clippy::too_many_arguments,
        reason = "one recorded HTTP exchange: method, target, request headers/body and response status/headers/body"
    )]
    pub(super) fn exchange(
        &mut self,
        date: &str,
        method: &str,
        url: &str,
        req_headers: &[(&str, &str)],
        req_body: &[u8],
        status: u16,
        resp_headers: &[(&str, &str)],
        resp_body: &[u8],
    ) {
        self.exchange_with_optional_date(
            Some(date),
            method,
            url,
            req_headers,
            req_body,
            status,
            resp_headers,
            resp_body,
        );
    }

    /// Appends a linked `request` + `response` pair whose `WARC-Date` header
    /// may be omitted so the HTTP `Date` header fallback is exercised.
    #[expect(
        clippy::too_many_arguments,
        reason = "one recorded HTTP exchange: method, target, request headers/body and response status/headers/body"
    )]
    pub(super) fn exchange_with_optional_date(
        &mut self,
        date: Option<&str>,
        method: &str,
        url: &str,
        req_headers: &[(&str, &str)],
        req_body: &[u8],
        status: u16,
        resp_headers: &[(&str, &str)],
        resp_body: &[u8],
    ) {
        let req_id = self.id();
        let resp_id = self.id();
        let parsed = url::Url::parse(url).expect("valid url");
        let mut target = parsed.path().to_owned();
        if let Some(query) = parsed.query() {
            target.push('?');
            target.push_str(query);
        }
        let host = parsed.host_str().unwrap_or_default();
        let mut req_block = format!("{method} {target} HTTP/1.1\r\nHost: {host}\r\n").into_bytes();
        for (name, value) in req_headers {
            req_block.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        req_block.extend_from_slice(b"\r\n");
        req_block.extend_from_slice(req_body);

        let mut resp_block = format!("HTTP/1.1 {status} OK\r\n").into_bytes();
        for (name, value) in resp_headers {
            resp_block.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        resp_block.extend_from_slice(b"\r\n");
        resp_block.extend_from_slice(resp_body);

        self.push_record(
            "request",
            &req_id,
            date,
            &[("WARC-Target-URI", url), ("WARC-Concurrent-To", &resp_id)],
            "application/http; msgtype=request",
            &req_block,
        );
        self.push_record(
            "response",
            &resp_id,
            date,
            &[("WARC-Target-URI", url), ("WARC-Concurrent-To", &req_id)],
            "application/http; msgtype=response",
            &resp_block,
        );
    }

    pub(super) fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

/// Starts a Run with `recording` (`mode: "replay"`) and `settings` with no
/// loopback `network.resolve` mappings so every host replays from the file
/// (ADR 0105, issue 801).
fn start_with_recording(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    settings: &Value,
    path: &Path,
) -> Result<Value, Gone> {
    let mut settings = site.mapped(settings, connection.acts_for_model());
    settings["network"]["resolve"] = json!({});
    let id = connection.request(
        RUN_START.name,
        &json!({
            "run_id": run_id,
            "settings": settings,
            "recording": { "mode": "replay", "path": path.to_string_lossy() },
        }),
    )?;
    Ok(connection.answer(id)?.message)
}

/// Finishes `run_id` and returns the Run report JSON.
fn finish_report(connection: &mut Connection, run_id: &str) -> Result<Value, Gone> {
    let id = connection.request(RUN_FINISH.name, &json!({ "run_id": run_id }))?;
    Ok(connection.browser_answer(id)?.message)
}

/// Reads the response body bytes of a `page.request` answer via
/// `response.body`.
#[expect(
    clippy::disallowed_methods,
    reason = "the synchronous conformance harness reads the kept body file on its own thread"
)]
fn request_body(
    connection: &mut Connection,
    page_id: &Value,
    answer: &Value,
) -> Result<Option<Vec<u8>>, Gone> {
    let Some(request_id) = answer["result"]["request_id"].as_str() else {
        return Ok(None);
    };
    let read = call(
        connection,
        RESPONSE_BODY.name,
        page_id,
        &json!({ "request_id": request_id }),
    )?;
    let Some(path) = read["result"]["path"].as_str() else {
        return Ok(None);
    };
    Ok(std::fs::read(path).ok())
}

/// Total requests the Proving ground has recorded so far.
fn total_received(site: &Site) -> usize {
    site.received_requests().every().count()
}

/// Recorded URLs and fixture file for [`replays_without_network`].
struct ReplayFixture {
    page_url: String,
    missing_url: String,
    login_url: String,
    vary_url: String,
    warc_path: PathBuf,
}

/// Builds the WARC fixture holding a page, its script subresource, repeated
/// login POST responses and `Vary: Accept-Language` responses.
fn build_replay_fixture(target: &EngineTarget, site: &Site) -> ReplayFixture {
    let page_url = site.named_url(HOST, "/replayed");
    let script_url = site.named_url(HOST, "/app.js");
    let missing_url = site.named_url(HOST, "/missing.png");
    let login_url = site.named_url(HOST, "/login");
    let vary_url = site.named_url(HOST, "/vary");

    let html = format!(
        "<!doctype html><html><body><script src=\"{script_url}\"></script><img src=\"{missing_url}\"></body></html>"
    );
    let script = b"document.documentElement.dataset.replayed = 'from-warc';";

    let mut warc = WarcBuilder::new("goodfetch/0.1");
    warc.exchange(
        "2026-01-05T09:00:01Z",
        "GET",
        &page_url,
        &[],
        b"",
        200,
        &[("Content-Type", "text/html")],
        html.as_bytes(),
    );
    warc.exchange(
        "2026-01-05T09:00:02Z",
        "GET",
        &script_url,
        &[],
        b"",
        200,
        &[("Content-Type", "application/javascript")],
        script,
    );
    warc.exchange(
        "2026-01-05T09:00:03Z",
        "POST",
        &login_url,
        &[("Content-Type", "application/json")],
        br#"{"pass":"pw","user":"ada"}"#,
        200,
        &[("Content-Type", "application/json")],
        br#"{"step":1}"#,
    );
    warc.exchange(
        "2026-01-05T09:00:04Z",
        "POST",
        &login_url,
        &[("Content-Type", "application/json")],
        br#"{"pass":"pw","user":"ada"}"#,
        200,
        &[("Content-Type", "application/json")],
        br#"{"step":2}"#,
    );
    warc.exchange(
        "2026-01-05T09:00:05Z",
        "GET",
        &vary_url,
        &[("Accept-Language", "en")],
        b"",
        200,
        &[("Content-Type", "text/plain"), ("Vary", "Accept-Language")],
        b"english",
    );
    warc.exchange(
        "2026-01-05T09:00:06Z",
        "GET",
        &vary_url,
        &[("Accept-Language", "fr")],
        b"",
        200,
        &[("Content-Type", "text/plain"), ("Vary", "Accept-Language")],
        b"french",
    );
    let warc_path = write_fixture(target, "replay-full.warc", &warc.finish());
    ReplayFixture {
        page_url,
        missing_url,
        login_url,
        vary_url,
        warc_path,
    }
}

/// Replays a document, its script subresource, a missing subresource,
/// repeated POST login calls with canonical JSON bodies and `Vary` headers,
/// verifying the Proving ground received zero requests and the Run report
/// lists the missing subresource.
fn replays_without_network(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let ReplayFixture {
        page_url,
        missing_url,
        login_url,
        vary_url,
        warc_path,
    } = build_replay_fixture(target, site);

    let before = total_received(site);
    let run_id = "conformance-recording-replay";
    let started = start_with_recording(connection, site, run_id, &json!({}), &warc_path)?;
    checks.check("the replaying Run starts", started.get("result").is_some());
    let Some(page) = new_page(connection, run_id, checks)? else {
        let _ = finish_report(connection, run_id)?;
        return Ok(());
    };
    let page_id = json!(page.page_id);

    // Fragment is stripped when matching.
    let goto_url = format!("{page_url}#section-1");
    let loaded = connection.goto(&page, &goto_url)?;
    answered(&loaded, "`goto` on replayed page", &page_url, 200, checks);
    let replayed = call(
        connection,
        PAGE_EVALUATE.name,
        &page_id,
        &json!({ "function": "() => document.documentElement.dataset.replayed ?? null" }),
    )?;
    checks.check(
        "the replayed script subresource ran from the WARC",
        replayed["result"]["value"] == json!("from-warc"),
    );

    // Repeated POST with reordered JSON keys (`{"user":"ada","pass":"pw"}`
    // in Base64) matches canonical body and replays in recorded order.
    let login_body = json!({
        "url": login_url,
        "method": "POST",
        "headers": { "content-type": "application/json" },
        "post_data": "eyJ1c2VyIjoiYWRhIiwicGFzcyI6InB3In0=",
    });
    let first = call(connection, PAGE_REQUEST.name, &page_id, &login_body)?;
    let first_body = request_body(connection, &page_id, &first)?;
    let second = call(connection, PAGE_REQUEST.name, &page_id, &login_body)?;
    let second_body = request_body(connection, &page_id, &second)?;
    checks.check(
        "the first repeated POST replays the first recorded response",
        first_body.as_deref() == Some(br#"{"step":1}"#.as_slice()),
    );
    checks.check(
        "the second repeated POST replays the second recorded response",
        second_body.as_deref() == Some(br#"{"step":2}"#.as_slice()),
    );

    // Vary header matching.
    let en = call(
        connection,
        PAGE_REQUEST.name,
        &page_id,
        &json!({ "url": vary_url, "headers": { "accept-language": "en" } }),
    )?;
    let en_body = request_body(connection, &page_id, &en)?;
    let fr = call(
        connection,
        PAGE_REQUEST.name,
        &page_id,
        &json!({ "url": vary_url, "headers": { "accept-language": "fr" } }),
    )?;
    let fr_body = request_body(connection, &page_id, &fr)?;
    checks.check(
        "`Vary: Accept-Language` en replays english",
        en_body.as_deref() == Some(b"english".as_slice()),
    );
    checks.check(
        "`Vary: Accept-Language` fr replays french",
        fr_body.as_deref() == Some(b"french".as_slice()),
    );

    let finished = finish_report(connection, run_id)?;
    let report = &finished["result"]["recording"];
    checks.check(
        "the report records mode `replay`",
        report["mode"] == json!("replay"),
    );
    checks.check(
        "the report counts all 6 used recorded responses",
        report["responses_used"] == json!(6),
    );
    checks.check(
        "the report names the software that wrote the WARC",
        report["written_by"] == json!("goodfetch/0.1"),
    );
    let missing_entry = format!("GET {missing_url}");
    let misses = report["misses"].as_array();
    checks.check(
        "the Run report lists the missing subresource",
        misses.is_some_and(|list| list.iter().any(|item| item == &json!(missing_entry))),
    );
    checks.check(
        "the Proving ground received zero requests during the replayed Run",
        total_received(site) == before,
    );
    Ok(())
}

/// A missing `goto` and a missing `page.request` each raise
/// `response_not_recorded` (not retryable) naming the method, URL,
/// recording and nearest recorded URLs, and both misses appear in the Run
/// report while the Proving ground receives zero requests.
fn miss_raises_and_reports(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let item1 = site.named_url(HOST, "/items?page=1");
    let item2 = site.named_url(HOST, "/items?page=2");
    let missing_nav = site.named_url(HOST, "/items?page=99");

    let mut warc = WarcBuilder::new("goodfetch/0.1");
    warc.exchange(
        "2026-01-05T09:00:01Z",
        "GET",
        &item1,
        &[],
        b"",
        200,
        &[("Content-Type", "text/html")],
        b"<p>1</p>",
    );
    warc.exchange(
        "2026-01-05T09:00:02Z",
        "GET",
        &item2,
        &[],
        b"",
        200,
        &[("Content-Type", "text/html")],
        b"<p>2</p>",
    );
    let warc_path = write_fixture(target, "replay-miss.warc", &warc.finish());
    let path_str = warc_path.to_string_lossy().into_owned();

    let before = total_received(site);
    let run_id = "conformance-recording-miss";
    let started = start_with_recording(connection, site, run_id, &json!({}), &warc_path)?;
    checks.check("the replaying Run starts", started.get("result").is_some());
    let Some(page) = new_page(connection, run_id, checks)? else {
        let _ = finish_report(connection, run_id)?;
        return Ok(());
    };
    let page_id = json!(page.page_id);

    let nav_err = call(
        connection,
        PAGE_GOTO.name,
        &page_id,
        &json!({ "url": missing_nav }),
    )?;
    let nav_data = &nav_err["error"]["data"];
    checks.check(
        "a missing `goto` raises `response_not_recorded`",
        nav_data["code"] == json!(RESPONSE_NOT_RECORDED.name),
    );
    checks.check(
        "`response_not_recorded` is not retryable",
        nav_data["retryable"] == json!(false),
    );
    checks.check(
        "the error details name method, URL, recording and nearest recorded URLs",
        nav_data["method"] == json!("GET")
            && nav_data["url"] == json!(missing_nav)
            && nav_data["recording"] == json!(path_str)
            && nav_data["nearest"] == json!([item1, item2]),
    );

    let req_err = call(
        connection,
        PAGE_REQUEST.name,
        &page_id,
        &json!({ "url": item1, "method": "POST" }),
    )?;
    let req_data = &req_err["error"]["data"];
    checks.check(
        "a `page.request` with an unrecorded method raises `response_not_recorded`",
        req_data["code"] == json!(RESPONSE_NOT_RECORDED.name)
            && req_data["method"] == json!("POST")
            && req_data["url"] == json!(item1),
    );

    let finished = finish_report(connection, run_id)?;
    let misses = &finished["result"]["recording"]["misses"];
    checks.check(
        "the Run report lists both misses",
        misses == &json!([format!("GET {missing_nav}"), format!("POST {item1}")]),
    );
    checks.check(
        "the Proving ground received zero requests",
        total_received(site) == before,
    );
    Ok(())
}

/// With `site_preferences.read_robots_txt_for_navigation` on, a replaying
/// Run reads robots.txt from the recording alone, and treats a missing
/// robots.txt as a listed miss that reads as no rules.
fn robots_txt_from_recording(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let robots_url = site.named_url(HOST, "/robots.txt");
    let secret_url = site.named_url(HOST, "/secret/doc");
    let open_url = site.named_url(CACHE_HOST, "/open");
    let missing_robots = site.named_url(CACHE_HOST, "/robots.txt");

    let mut warc = WarcBuilder::new("goodfetch/0.1");
    warc.exchange(
        "2026-01-05T09:00:01Z",
        "GET",
        &robots_url,
        &[],
        b"",
        200,
        &[("Content-Type", "text/plain")],
        b"User-agent: *\r\nDisallow: /secret/\r\n",
    );
    warc.exchange(
        "2026-01-05T09:00:02Z",
        "GET",
        &secret_url,
        &[],
        b"",
        200,
        &[("Content-Type", "text/html")],
        b"<p>secret</p>",
    );
    warc.exchange(
        "2026-01-05T09:00:03Z",
        "GET",
        &open_url,
        &[],
        b"",
        200,
        &[("Content-Type", "text/html")],
        b"<p>open</p>",
    );
    let warc_path = write_fixture(target, "replay-robots.warc", &warc.finish());

    let before = total_received(site);
    let run_id = "conformance-recording-robots";
    let settings = json!({ "site_preferences": { "read_robots_txt_for_navigation": true } });
    let started = start_with_recording(connection, site, run_id, &settings, &warc_path)?;
    checks.check("the replaying Run starts", started.get("result").is_some());
    let Some(page) = new_page(connection, run_id, checks)? else {
        let _ = finish_report(connection, run_id)?;
        return Ok(());
    };

    let loaded_secret = connection.goto(&page, &secret_url)?;
    answered(
        &loaded_secret,
        "`goto` to /secret/doc",
        &secret_url,
        200,
        checks,
    );
    let loaded_open = connection.goto(&page, &open_url)?;
    answered(&loaded_open, "`goto` to /open", &open_url, 200, checks);

    let entries = checks.run_trace(poll::try_until(
        RECORDED_WITHIN,
        Duration::from_millis(100),
        || {
            Ok(run_trace::provenance(target, run_id)?
                .into_iter()
                .filter(|entry| entry["action"] == json!(ROBOTS_ACTION))
                .collect::<Vec<_>>())
        },
        |read| read.len() >= 2,
    ));
    let secret_entry = entries
        .iter()
        .find(|entry| entry["url"] == json!(secret_url));
    let open_entry = entries.iter().find(|entry| entry["url"] == json!(open_url));
    checks.check(
        "robots.txt from the recording marks /secret/doc disallowed",
        secret_entry.is_some_and(|entry| {
            entry["file"] == json!("found") && entry["allowed"] == json!(false)
        }),
    );
    checks.check(
        "a missing robots.txt in the recording reads as no rules (allowed)",
        open_entry.is_some_and(|entry| {
            entry["file"] == json!("not_found") && entry["allowed"] == json!(true)
        }),
    );

    let finished = finish_report(connection, run_id)?;
    let missing_entry = format!("GET {missing_robots}");
    let misses = finished["result"]["recording"]["misses"].as_array();
    checks.check(
        "the missing robots.txt is listed in the Run report's misses",
        misses.is_some_and(|list| list.iter().any(|item| item == &json!(missing_entry))),
    );
    checks.check(
        "the Proving ground received zero requests",
        total_received(site) == before,
    );
    Ok(())
}

/// An unreadable file, a non-WARC file and an unsupported WARC version each
/// fail `run.start` with `recording_invalid`.
fn invalid_recording_fails_start(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let missing = target
        .environment
        .get("GOODFETCH_HOME")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("recordings")
        .join("does-not-exist.warc");
    let not_warc = write_fixture(target, "not-warc.warc", b"hello, not warc\r\n");
    let bad_version = write_fixture(
        target,
        "warc-2-0.warc",
        b"WARC/2.0\r\nWARC-Type: warcinfo\r\nContent-Length: 0\r\n\r\n\r\n\r\n",
    );

    for (label, run_id, path) in [
        ("unreadable file", "conformance-rec-bad-missing", missing),
        ("non-WARC file", "conformance-rec-bad-not-warc", not_warc),
        (
            "unsupported WARC version",
            "conformance-rec-bad-version",
            bad_version,
        ),
    ] {
        let reply = start_with_recording(connection, site, run_id, &json!({}), &path)?;
        let data = &reply["error"]["data"];
        checks.check(
            format!("{label} fails `run.start` with `recording_invalid`"),
            data["code"] == json!(RECORDING_INVALID.name)
                && data["retryable"] == json!(false)
                && data["recording"] == json!(path.to_string_lossy())
                && data["reason"]
                    .as_str()
                    .is_some_and(|reason| !reason.is_empty()),
        );
    }
    Ok(())
}
