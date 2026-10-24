//! The Provenance a program's own navigation records of robots.txt (ADRs
//! 0086 and 0098; spec #580): for each document a page a program opened
//! loads, whether its origin's robots.txt allowed it, naming the group and
//! the rule that decided. The groups are those area 5's selection picks
//! for Foxhound's product token, with no operator token, since a Collie
//! Run takes none; a robots.txt answered `4xx` has no rules, and one
//! answered `429` or `5xx`, or not at all, disallows everything, as RFC
//! 9309 §2.3.1 reads them. The reading gates, delays and counts nothing:
//! it starts once the document loaded, as a task of the Run, and the Run's
//! own copy fetches each origin's file once, through the page's Session as
//! `page.request` sends (the browser-fetch tier), never through the copy
//! Foxhound shares.

use goodfetch_engine::robots_txt::{self, Fetched, Judgment, RobotsTxt};
use goodfetch_protocol::page::{PageId, SessionId};
use serde_json::{Value, json};
use url::Url;

use crate::browser_fetch::{self, Body};
use crate::interception::EngineRequests;
use crate::page_value::PageValue;
use crate::robots_txt::{Group, applicable_rules};
use crate::sessions::{OpenPage, Sessions};
use crate::trace;

/// How many redirects a robots.txt fetch follows: RFC 9309 §2.3.1.2's
/// least.
const REDIRECTS: u32 = 5;

/// Starts reading the robots.txt of `url`'s origin for the document
/// `page_id`, a page a program opened, loaded at `url`, as a task of its
/// Run that records the reading in Provenance; a URL that is not HTTP's
/// starts none. The page's navigation never waits for it.
pub(super) fn read(
    sessions: &Sessions,
    steps: EngineRequests,
    page_id: &PageId,
    page: &OpenPage,
    url: &str,
) {
    let Ok(url) = Url::parse(url) else {
        return;
    };
    let Some(location) = robots_txt::location(&url) else {
        return;
    };
    let sessions = sessions.clone();
    let page_id = page_id.clone();
    let session = page.session.clone();
    let value = page.driver.value().clone();
    let run = page.run.clone();
    page.run.spawn(async move {
        let fetched = run
            .robots_txt()
            .copy(&location, || {
                fetch(&sessions, &steps, &session, &value, &location)
            })
            .await;
        let reading = Reading {
            page_id: &page_id,
            session: &session,
            url: &url,
            location: &location,
        };
        trace::provenance(&run, &reading.entry(&fetched)).await;
    });
}

/// Fetches `location` from a page of the Session `session` the engine
/// opens for it and closes after, as `page.request` sends a request.
async fn fetch(
    sessions: &Sessions,
    steps: &EngineRequests,
    session: &SessionId,
    value: &PageValue,
    location: &Url,
) -> Fetched {
    let (driver, in_browser) = match sessions.engine_page(session, value).await {
        Ok(opened) => opened,
        Err(error) => return Fetched::Unreachable(error.message().to_owned()),
    };
    let request = browser_fetch::Request {
        url: location.as_str(),
        method: None,
        headers: None,
        post_data: None,
        max_redirects: REDIRECTS,
        option: "robots.txt",
    };
    let fetched = browser_fetch::send(&driver, steps, &request).await;
    driver.close().await;
    drop(in_browser);
    match fetched {
        Ok(fetched) => match fetched.body {
            Body::Whole(bytes) => Fetched::of_response(fetched.response.status, &bytes),
            Body::Cut { .. } => Fetched::Unreachable("its body was cut".to_owned()),
        },
        Err(error) => Fetched::Unreachable(error.message().to_owned()),
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
    /// robots.txt as the Run's copy holds it.
    fn entry(&self, fetched: &Fetched) -> Value {
        let mut entry = json!({
            "action": ROBOTS_TXT,
            "page_id": self.page_id,
            "session_id": self.session,
            "url": self.url.as_str(),
            "robots_txt": self.location.as_str(),
            "group": null,
            "rule": null,
        });
        match fetched {
            Fetched::File(file) => {
                entry["file"] = json!("found");
                let (allowed, decided) = judged(file, self.url);
                entry["allowed"] = json!(allowed);
                if let Some((group, rule)) = decided {
                    entry["group"] = group;
                    entry["rule"] = rule;
                }
            }
            Fetched::NotFound(status) => {
                entry["file"] = json!("not_found");
                entry["status"] = json!(status);
                entry["allowed"] = json!(true);
            }
            Fetched::Unreachable(reason) => {
                entry["file"] = json!("unreachable");
                entry["reason"] = json!(reason);
                entry["allowed"] = json!(false);
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
