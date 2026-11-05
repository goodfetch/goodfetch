//! The Provenance a program's own navigation records of robots.txt (ADRs
//! 0086 and 0098; spec #580): for each document the main frame of a page a
//! program opened receives from the site, whatever started its navigation
//! (a `goto`, a click, a form, a script, a popup), and for each response a
//! `page.request` of such a page gets, whether its origin's robots.txt
//! allowed it, naming the group and the rule that decided. The groups are
//! those area 5's selection picks for Foxhound's product token, with no
//! operator token, since a Collie Run takes none; a robots.txt answered
//! `4xx` has no rules, and one answered `429` or `5xx`, or not at all,
//! disallows everything, as RFC 9309 §2.3.1 reads them. The reading gates,
//! delays and counts nothing: it starts once the document's response
//! arrived, as a task of the Run, and the Run's own copy fetches each
//! origin's file once, through the page's Session as `page.request` sends
//! (the browser-fetch tier), never through the copy Foxhound shares. Its
//! body is read from the browser's stream, judged as it arrives and stopped
//! at the Run's limits (SC-05), so a file that decodes past
//! `network.max_body_size` is unreachable, never held whole.
//!
//! A reading whose fetch never reached the site, because the Session it
//! reads through closed, its lease ended or its browser went first, is no
//! answer of the site's: the copy keeps nothing of it, so the next document
//! of the origin fetches again, and Provenance records it `not_read`, with
//! its reason and no verdict.

use std::sync::Arc;

use goodfetch_engine::response_limits::Limits;
use goodfetch_engine::robots_txt::{self, Answer, Fetched, Judgment, NotRead, RobotsTxt};
use goodfetch_engine::runs::Run;
use goodfetch_protocol::page::{PageId, SessionId};
use serde_json::{Value, json};
use url::Url;

use super::Sessions;
use crate::browser_fetch::{self, Body};
use crate::interception::{EngineRequests, OnDocument};
use crate::page_value::PageValue;
use crate::robots_txt::{Group, applicable_rules};
use crate::trace;

/// How many redirects a robots.txt fetch follows: RFC 9309 §2.3.1.2's
/// least.
const REDIRECTS: u32 = 5;

/// Where the documents of one page a program opened are read for
/// robots.txt: its Session, its value and its Run.
#[derive(Clone)]
pub(crate) struct Reader {
    sessions: Sessions,
    page_id: PageId,
    session: SessionId,
    value: PageValue,
    run: Run,
}

impl OnDocument for Reader {
    fn received(&self, url: &str, steps: &EngineRequests) {
        self.read(steps.clone(), url);
    }
}

impl Reader {
    /// The reader of the page `page_id` of the Session `session`, which
    /// `value` carries in `run`, when a program opened it: a handler's page,
    /// whose value has a navigation gate, is Foxhound's to judge (ADR 0087).
    pub(crate) fn of(
        sessions: &Sessions,
        page_id: &PageId,
        session: &SessionId,
        value: &PageValue,
        run: &Run,
    ) -> Option<Self> {
        value.gate().is_none().then(|| Self {
            sessions: sessions.clone(),
            page_id: page_id.clone(),
            session: session.clone(),
            value: value.clone(),
            run: run.clone(),
        })
    }

    /// Starts reading the robots.txt of `url`'s origin for the page's
    /// document or response at `url`, as a task of its Run that records the
    /// reading in Provenance, the engine's own requests sent through
    /// `steps`; a URL that is not HTTP's starts none. Nothing the page does
    /// waits for it.
    pub(crate) fn read(&self, steps: EngineRequests, url: &str) {
        let Ok(url) = Url::parse(url) else {
            return;
        };
        let Some(location) = robots_txt::location(&url) else {
            return;
        };
        let reader = self.clone();
        let limits = self
            .run
            .settings()
            .map_or_else(Limits::default, |settings| Limits::of(settings.resolved()));
        self.run.spawn(async move {
            // Boxed: the fetch holds a browser load's state (clippy's
            // `large_futures`).
            let fetched = Box::pin(
                reader
                    .run
                    .own_robots_txt(&location, reader.fetch(&steps, &location, limits)),
            )
            .await;
            let reading = Reading {
                page_id: &reader.page_id,
                session: &reader.session,
                url: &url,
                location: &location,
            };
            trace::provenance(&reader.run, reading.entry(&fetched)).await;
        });
    }

    /// Fetches `location` from a page of the Session the engine opens for it
    /// and closes after, as `page.request` sends a request, its body under
    /// the Run's `limits` (SC-05): [`NotRead`] when no request of it
    /// reached the site, since the Session no longer served the reading.
    async fn fetch(
        &self,
        steps: &EngineRequests,
        location: &Url,
        limits: Limits,
    ) -> Result<Answer, NotRead> {
        // A navigation call cancelled meanwhile closes the engine's page as
        // it drops.
        let engine_page = self
            .sessions
            .engine_page(&self.session, &self.value)
            .await
            .map_err(|error| NotRead(error.message().to_owned()))?;
        let request = browser_fetch::Request {
            url: location.as_str(),
            method: None,
            headers: None,
            post_data: None,
            max_redirects: REDIRECTS,
            option: "robots.txt",
            limits,
            read: browser_fetch::BodyRead::Streamed,
        };
        let fetched = browser_fetch::send(engine_page.driver(), steps, &request).await;
        engine_page.close().await;
        let failed = match fetched {
            Ok(fetched) => match fetched.body {
                Body::Whole(bytes) => {
                    return Ok(Answer::Response {
                        status: fetched.response.status,
                        body: bytes.iter().copied().take(robots_txt::MAX_SIZE).collect(),
                    });
                }
                Body::Cut { .. } => "its body was cut".to_owned(),
            },
            Err(error) => error.message().to_owned(),
        };
        // A fetch the Session's end, its lease's or its browser's cut short
        // failed in the engine, not at the site: the browser aborts its
        // page's load as its context goes.
        if self.sessions.serves(&self.session, &self.value).await {
            Ok(Answer::Unreachable(failed))
        } else {
            Err(NotRead(failed))
        }
    }
}

/// The Provenance action of a robots.txt reading.
const ROBOTS_TXT: &str = "robots_txt";

/// The page whose document a reading names.
struct Reading<'a> {
    page_id: &'a PageId,
    session: &'a SessionId,
    /// The document's URL.
    url: &'a Url,
    /// Its origin's robots.txt.
    location: &'a Url,
}

impl Reading<'_> {
    /// The Provenance entry of the document under `fetched`, its origin's
    /// robots.txt as the Run's copy holds it, or why it was not read.
    fn entry(&self, fetched: &Result<Arc<Fetched>, NotRead>) -> Value {
        let mut entry = json!({
            "action": ROBOTS_TXT,
            "page_id": self.page_id,
            "session_id": self.session,
            "url": self.url.as_str(),
            "robots_txt": self.location.as_str(),
            "group": null,
            "rule": null,
        });
        match fetched.as_deref() {
            Ok(Fetched::File(file)) => {
                entry["file"] = json!("found");
                let (allowed, decided) = judged(file, self.url);
                entry["allowed"] = json!(allowed);
                if let Some((group, rule)) = decided {
                    entry["group"] = group;
                    entry["rule"] = rule;
                }
            }
            Ok(Fetched::NotFound(status)) => {
                entry["file"] = json!("not_found");
                entry["status"] = json!(status);
                entry["allowed"] = json!(true);
            }
            Ok(Fetched::Unreachable(reason)) => {
                entry["file"] = json!("unreachable");
                entry["reason"] = json!(reason);
                entry["allowed"] = json!(false);
            }
            Err(NotRead(reason)) => {
                entry["file"] = json!("not_read");
                entry["reason"] = json!(reason);
            }
        }
        entry
    }
}

/// Whether `file` allows `url`, and the group and rule that decided, as
/// Provenance names them, when a rule matched.
fn judged(file: &RobotsTxt, url: &Url) -> (bool, Option<(Value, Value)>) {
    let groups: Vec<Group<&robots_txt::Rule>> = file
        .groups()
        .iter()
        .map(|group| Group {
            user_agents: group.user_agents().to_vec(),
            rules: group.rules().iter().collect(),
        })
        .collect();
    let rules = applicable_rules(&groups, None);
    let (allowed, rule) =
        match robots_txt::judge(rules.into_iter().copied(), &robots_txt::target(url)) {
            Judgment::Allowed(rule) => (true, rule),
            Judgment::Disallowed(rule) => (false, Some(rule)),
        };
    let decided = rule.and_then(|rule| {
        let group = file.groups().get(rule.group())?;
        Some((
            json!({ "user_agents": group.user_agents(), "line": group.line() }),
            json!({ "text": rule.to_string(), "line": rule.line() }),
        ))
    });
    (allowed, decided)
}
