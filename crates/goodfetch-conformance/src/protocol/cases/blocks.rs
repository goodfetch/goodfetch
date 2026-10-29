//! The page suite's Block cause cases (ADRs 0088, 0092, 0098 and 0118,
//! ticket #711): on a Session a program or an Agent opened, a Block cause
//! raises `blocked` with the cause, the URL, the status and `retry_after`,
//! from the call that waited for the navigation that met it, or else from
//! the page's next call; the Session stays open with its cookies, never
//! replaced (one Session in the Run's Provenance, and no browser of its CDP
//! trace given a second browser context), and nothing is written to
//! Politeness's domain limits. Every page of the Proving ground's
//! block-detection corpus is met by a Session in the engine, as the
//! in-process corpus cases classify it, and the access-policy pages open no
//! second Session. `page.request` judges no Block cause.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::page::request::PAGE_REQUEST;
use goodfetch_protocol::page::session_pages::SESSION_PAGES;
use goodfetch_protocol::page::waits::{PAGE_WAIT_FOR_FUNCTION, PAGE_WAIT_FOR_URL};
use goodfetch_protocol::page::{BLOCKED, BlockedDetails, LOCATOR_CLICK, PAGE_GOTO, PageOpened};
use serde_json::{Value, json};

use super::clean_up::{provenance, trace_lines};
use super::navigation::url_on;
use super::{Checks, Site, finish_run, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::hello_as;
use crate::protocol::target::EngineTarget;

/// The access family's task whose start page is the block-detection
/// corpus's index.
const CORPUS_TASK: &str = "access/5";

/// The wait the Proving ground's `/retry-after/<status>/<form>/long` asks
/// for.
const RETRY_AFTER_LONG: Duration = Duration::from_hours(1);

/// The CDP command that opens a Session's browser context.
const NEW_CONTEXT: &str = "Target.createBrowserContext";

/// The CDP command the engine sends a browser first, once it connected to
/// it: one for each browser of the Run.
const CONNECTED: &str = "Browser.getVersion";

/// One Block cause case of the page suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    RateLimit,
    NextCall,
    Corpus,
    Owners,
    AccessPolicy,
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-pages-429-raises-blocked-with-its-retry-after-and-the-session-keeps-its-cookies-for-the-next-navigation" => {
                Self::RateLimit
            }
            "a-403-behind-a-block-cause-raises-blocked-from-the-goto-or-else-from-the-pages-next-call" => {
                Self::NextCall
            }
            "each-page-of-the-block-detection-corpus-a-session-meets-raises-blocked-with-its-cause-or-answers-its-response" => {
                Self::Corpus
            }
            "a-block-never-replaces-the-programs-or-the-agents-session" => Self::Owners,
            "the-access-policy-pages-open-no-second-session-and-write-no-domain-limit" => {
                Self::AccessPolicy
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
            Self::RateLimit => rate_limit(connection, target, site, checks),
            Self::NextCall => next_call(connection, target, site, checks),
            Self::Corpus => corpus(connection, site, checks),
            Self::Owners => owners(connection, target, site, checks),
            Self::AccessPolicy => access_policy(connection, target, site, checks),
        }
    }
}

/// Calls `method` with `params` and answers the engine's whole answer,
/// waiting as long as a browser's launch may take.
fn call(connection: &mut Connection, method: &str, params: &Value) -> Result<Value, Gone> {
    let id = connection.request(method, params)?;
    Ok(connection.browser_answer(id)?.message)
}

/// `page.goto` of `page` to `url`.
fn goto(connection: &mut Connection, page: &PageOpened, url: &str) -> Result<Value, Gone> {
    call(
        connection,
        PAGE_GOTO.name,
        &json!({ "page_id": page.page_id, "url": url }),
    )
}

/// `page.evaluate` of a function that reads the page's title.
fn read_title(connection: &mut Connection, page: &PageOpened) -> Result<Value, Gone> {
    call(
        connection,
        PAGE_EVALUATE.name,
        &json!({ "page_id": page.page_id, "function": "() => document.title" }),
    )
}

/// The details of `answer` when it raised `blocked`.
fn blocked(answer: &Value) -> Option<BlockedDetails> {
    let data = &answer["error"]["data"];
    if data["code"] != json!(BLOCKED.name) {
        return None;
    }
    serde_json::from_value(data.clone()).ok()
}

/// Whether `answer` raised `blocked` for `cause` on the document at a URL
/// ending with `path`, answered with `status`.
fn raised(answer: &Value, cause: &str, path: &str, status: u16) -> bool {
    blocked(answer).is_some_and(|details| {
        details.cause.as_str() == cause
            && details.url.ends_with(path)
            && details.status == Some(status)
    })
}

/// Whether `answer` is the document's response, with `status`.
fn answered(answer: &Value, status: u16) -> bool {
    answer["result"]["response"]["status"] == json!(status)
}

/// The wall time now, which a `retry_after` is measured against.
#[expect(
    clippy::disallowed_methods,
    reason = "the harness measures the engine's answer against the machine's own time; it runs no Clock"
)]
fn now() -> SystemTime {
    SystemTime::now()
}

/// Whether the Run `run_id` kept one Session throughout: its Provenance
/// names one Session, and its CDP trace shows no browser given a second
/// browser context. A Session moves to another browser when the browser
/// pool recycles its own, or the Resource governor sheds it while memory is
/// critical, and opens its context there with its cookies (ADR 0040), so
/// each browser holds the same Session; a browser launched for a move that
/// the Run's end cut short holds none. A Session opened in place of a
/// blocked one names another Session ID and makes a second context in a
/// browser already holding one.
fn kept_one_session(target: &EngineTarget, run_id: &str) -> bool {
    let commands = trace_lines(target, run_id, "trace/cdp-commands.jsonl");
    let sent = |method: &str| {
        commands
            .iter()
            .filter(|line| line["method"] == json!(method))
            .count()
    };
    let contexts = sent(NEW_CONTEXT);
    let sessions: BTreeSet<String> = provenance(target, run_id)
        .iter()
        .filter_map(|entry| entry["session_id"].as_str().map(str::to_owned))
        .collect();
    contexts > 0 && contexts <= sent(CONNECTED) && sessions.len() == 1
}

/// Whether the Politeness file of `target`'s Goodfetch home holds anything
/// naming `host`: a domain limit, a Permit or a waiter of its key. A file
/// never made holds nothing.
pub(super) fn politeness_names(target: &EngineTarget, host: &str) -> bool {
    let Some(home) = target.environment.get("GOODFETCH_HOME") else {
        return false;
    };
    let file = Path::new(home).join("politeness.sqlite");
    if !file.is_file() {
        return false;
    }
    let Ok(connection) =
        rusqlite::Connection::open_with_flags(&file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return true;
    };
    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()
        })
        .unwrap_or_default();
    tables.iter().any(|table| {
        let query = format!("SELECT * FROM \"{table}\"");
        let Ok(mut statement) = connection.prepare(&query) else {
            return true;
        };
        let columns = statement.column_count();
        let Ok(mut rows) = statement.query([]) else {
            return true;
        };
        while let Ok(Some(row)) = rows.next() {
            for column in 0..columns {
                if let Ok(rusqlite::types::ValueRef::Text(text)) = row.get_ref(column)
                    && String::from_utf8_lossy(text).contains(host)
                {
                    return true;
                }
            }
        }
        false
    })
}

/// The `scope.test` storage page with `mark`, which sets a cookie of the
/// Session the first time.
fn storage(site: &Site, mark: &str) -> String {
    site.named_url("scope.test", &format!("/storage?mark={mark}"))
        .unwrap_or_default()
}

/// The cookies the Proving ground received with each request for the
/// storage page with `mark`, in arrival order.
fn storage_cookies(site: &Site, mark: &str) -> Vec<Value> {
    let target = format!("/storage?mark={mark}");
    site.proving_ground
        .received_requests()
        .unwrap_or_default()
        .of_the_case()
        .filter(|request| {
            request["host"] == json!("scope.test") && request["target"] == json!(target)
        })
        .map(|request| request["cookies"].clone())
        .collect()
}

/// A Collie page's 429 with a `Retry-After` of an hour, in seconds and as
/// an HTTP-date measured against the response's own `Date`, raises
/// `blocked` from its `goto` with `rate_limit`, the URL, the status and the
/// time the site asked for; nothing waits on it, so the same URL loads again
/// before that time, and the Session keeps its cookie for the next navigation, the Run
/// keeping that one Session and writing nothing to Politeness (Fetch edge cases,
/// Collie's lines).
fn rate_limit(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-blocked-rate-limit";
    let mark = "rate-limit";
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let stored = goto(connection, &page, &storage(site, mark))?;
    checks.check(
        "the storage page sets the Session's cookie",
        answered(&stored, 200),
    );
    for form in ["seconds", "imf-fixdate"] {
        // An hour's wait: a `goto` that waited for it would outlast its
        // navigation timeout, so no load can make one that did not wait
        // look as if it had.
        let path = format!("/retry-after/429/{form}/long");
        let url = site.named_url("fetch-edge.test", &path).unwrap_or_default();
        let before = now();
        let went = goto(connection, &page, &url)?;
        let after = now();
        let asked = blocked(&went)
            .and_then(|details| details.retry_after)
            .map(SystemTime::from);
        checks.check(
            format!("a 429 whose `Retry-After` is in {form} raises `blocked` from its `goto` with `rate_limit`, its URL and status"),
            raised(&went, "rate_limit", &path, 429),
        );
        checks.check(
            format!("its `retry_after` is the time the site asked for: an hour after the response's own `Date`, from the moment it arrived ({form})"),
            asked.is_some_and(|asked| {
                before + RETRY_AFTER_LONG <= asked && asked <= after + RETRY_AFTER_LONG
            }),
        );
        let again = goto(connection, &page, &url)?;
        checks.check(
            format!(
                "nothing waits on it: the same URL loads on the same page before that time ({form})"
            ),
            answered(&again, 200) && asked.is_some_and(|asked| now() < asked),
        );
    }
    let back = goto(connection, &page, &storage(site, mark))?;
    checks.check(
        "the next navigation works and sends the Session's own cookie",
        answered(&back, 200)
            && storage_cookies(site, mark) == vec![json!([]), json!([["mark", mark]])],
    );
    finish_run(connection, run_id)?;
    checks.check(
        "the Run kept one Session: it was never replaced",
        kept_one_session(target, run_id),
    );
    checks.check(
        "nothing in the Politeness file names the site's domain",
        !politeness_names(target, "fetch-edge.test"),
    );
    Ok(())
}

/// A 403 behind a Block cause raises `blocked` from the `goto` that met it;
/// a click on a link to another such page waits for no navigation, so the
/// page's next call raises it, once, and the call after answers; a
/// `waitForURL` that waits for such a navigation raises it itself; and the
/// page stays the Session's.
fn next_call(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-blocked-next-call";
    let (Some(ip_block), Some(index)) = (
        url_on(site, CORPUS_TASK, "/corpus/ip-block"),
        url_on(site, CORPUS_TASK, "/corpus"),
    ) else {
        checks.check(
            "the Proving ground serves the block-detection corpus",
            false,
        );
        return Ok(());
    };
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let went = goto(connection, &page, &ip_block)?;
    checks.check(
        "a 403 that says the address is blocked raises `blocked` from its `goto` with `ip_block`",
        raised(&went, "ip_block", "/corpus/ip-block", 403)
            && blocked(&went).is_some_and(|details| details.retry_after.is_none()),
    );
    let listed = goto(connection, &page, &index)?;
    checks.check(
        "the page goes on to the corpus's index",
        answered(&listed, 200),
    );
    let clicked = call(
        connection,
        LOCATOR_CLICK.name,
        &json!({ "locator": { "page_id": page.page_id, "selector": "a[href=\"/corpus/cloudflare-block\"]" } }),
    )?;
    checks.check(
        "the click on the link answers: it waits for no navigation",
        clicked.get("result").is_some(),
    );
    // The page's next call is the wait, when the refusal's document
    // committed before it came, and else the call after the wait, which
    // answers once the document is there.
    let arrived = call(
        connection,
        PAGE_WAIT_FOR_FUNCTION.name,
        &json!({
            "page_id": page.page_id,
            "function": "() => location.pathname === '/corpus/cloudflare-block' && document.readyState === 'complete'",
        }),
    )?;
    let next = if arrived.get("result").is_some() {
        read_title(connection, &page)?
    } else {
        arrived
    };
    checks.check(
        "the page's next call once the refusal's document is there raises `blocked` with `ip_block`, the URL and the status",
        raised(&next, "ip_block", "/corpus/cloudflare-block", 403),
    );
    let after = read_title(connection, &page)?;
    checks.check(
        "it raises it once: the call after answers",
        after["result"].is_object(),
    );
    goto(connection, &page, &index)?;
    let clicked = call(
        connection,
        LOCATOR_CLICK.name,
        &json!({ "locator": { "page_id": page.page_id, "selector": "a[href=\"/corpus/incapsula-block\"]" } }),
    )?;
    let waited = call(
        connection,
        PAGE_WAIT_FOR_URL.name,
        &json!({ "page_id": page.page_id, "url": { "kind": "glob", "pattern": "**/corpus/incapsula-block" } }),
    )?;
    checks.check(
        "a `waitForURL` that waits for such a navigation raises `blocked` itself",
        clicked.get("result").is_some()
            && raised(&waited, "ip_block", "/corpus/incapsula-block", 403),
    );
    let pages = call(
        connection,
        SESSION_PAGES.name,
        &json!({ "session_id": page.session_id }),
    )?;
    checks.check(
        "the page is still its Session's",
        pages["result"]["pages"] == json!([page.page_id]),
    );
    finish_run(connection, run_id)?;
    checks.check("the Run kept one Session", kept_one_session(target, run_id));
    Ok(())
}

/// The corpus pages, each with the Block cause a Session meeting it
/// raises, or `None` for a page that answers its response, and its status.
const CORPUS: [(&str, Option<&str>, u16); 12] = [
    ("/corpus/interstitial", Some("challenge"), 403),
    ("/corpus/captcha", Some("captcha"), 200),
    ("/corpus/rate-limit", Some("rate_limit"), 429),
    ("/corpus/ip-block", Some("ip_block"), 403),
    ("/corpus/press-and-hold", Some("captcha"), 403),
    ("/corpus/cloudflare-block", Some("ip_block"), 403),
    ("/corpus/incapsula-block", Some("ip_block"), 403),
    ("/corpus/sucuri-block", Some("ip_block"), 403),
    // `decoy` is Foxhound's Frontier's alone (ADR 0095).
    ("/corpus/labyrinth", None, 200),
    ("/corpus/vendor-script", None, 200),
    ("/corpus/not-found", None, 404),
    ("/corpus/article", None, 200),
];

/// Every page of the block-detection corpus a Session meets raises
/// `blocked` with the cause its corpus records, or answers its response when
/// it is no block: a vendor's script on an ordinary page, an article and a
/// 404 are none (the in-process corpus cases' pages, met in the engine).
fn corpus(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-blocked-corpus";
    if site.start_url(CORPUS_TASK).is_none() {
        checks.check(
            "the Proving ground serves the block-detection corpus",
            false,
        );
        return Ok(());
    }
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    for (path, cause, status) in CORPUS {
        let url = url_on(site, CORPUS_TASK, path).unwrap_or_default();
        let went = goto(connection, &page, &url)?;
        match cause {
            Some(cause) => checks.check(
                format!("`{path}` raises `blocked` with `{cause}` and {status}"),
                raised(&went, cause, path, status),
            ),
            None => checks.check(
                format!("`{path}` is no block: it answers its response, {status}"),
                answered(&went, status),
            ),
        }
    }
    let rate_limited = url_on(site, CORPUS_TASK, "/corpus/rate-limit").unwrap_or_default();
    let before = now();
    let went = goto(connection, &page, &rate_limited)?;
    checks.check(
        "the corpus's rate limit names the two minutes its `Retry-After` asks for",
        blocked(&went)
            .and_then(|details| details.retry_after)
            .map(SystemTime::from)
            .is_some_and(|asked| {
                asked.duration_since(before).is_ok_and(|wait| {
                    wait >= Duration::from_secs(110) && wait <= Duration::from_secs(150)
                })
            }),
    );
    finish_run(connection, run_id)
}

/// On the program's Session and on the Session an Agent opens through the
/// MCP server alike: a challenge, a 403 and a 429 each raise `blocked` on
/// the `goto` that met them, `page.request` answers the 429's response, and
/// the same page then loads an article, its Run keeping that one Session:
/// no block retires or replaces it.
fn owners(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    if site.start_url(CORPUS_TASK).is_none() {
        checks.check(
            "the Proving ground serves the block-detection corpus",
            false,
        );
        return Ok(());
    }
    meets_blocks(connection, target, site, "program", checks)?;
    let mut agent = connection
        .another(target)
        .map_err(|error| Gone::Failed(error.to_string()))?;
    let opened = hello_as(&mut agent, Product::Collie, Surface::Mcp);
    checks.check(
        "an Agent's Client, through the MCP server, opens its own Connection",
        opened.is_ok(),
    );
    // A Client acting for a model never lists private ranges: user
    // configuration lists the site's hosts while its Run goes.
    let major = target
        .environment
        .get("GOODFETCH_HOME")
        .and_then(|home| super::consent::walls::major_of(home, "conformance-blocked-program"));
    checks.check("the engine's per-major folder is found", major.is_some());
    let before = major
        .as_deref()
        .and_then(|major| super::consent::walls::configure(major, Some(&site.configured_hosts())));
    let met = if opened.is_ok() {
        meets_blocks(&mut agent, target, site, "agent", checks)
    } else {
        Ok(())
    };
    if let Some(major) = &major {
        super::consent::walls::configure(major, before.as_deref());
    }
    connection.keep(agent.take_exchanged());
    agent.close();
    met
}

/// The challenge, the 403 and the 429 met by a Session of `owner`'s Run on
/// `connection`.
fn meets_blocks(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    owner: &str,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = format!("conformance-blocked-{owner}");
    start_run(connection, site, &run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, &run_id, checks)? else {
        return finish_run(connection, &run_id);
    };
    for (path, cause, status) in [
        ("/corpus/interstitial", "challenge", 403),
        ("/corpus/ip-block", "ip_block", 403),
        ("/corpus/rate-limit", "rate_limit", 429),
    ] {
        let url = url_on(site, CORPUS_TASK, path).unwrap_or_default();
        let went = goto(connection, &page, &url)?;
        checks.check(
            format!(
                "on the {owner}'s Session, `{path}` raises `blocked` with `{cause}` from its `goto`"
            ),
            raised(&went, cause, path, status),
        );
    }
    let rate_limited = url_on(site, CORPUS_TASK, "/corpus/rate-limit").unwrap_or_default();
    let requested = call(
        connection,
        PAGE_REQUEST.name,
        &json!({ "page_id": page.page_id, "url": rate_limited }),
    )?;
    checks.check(
        format!(
            "on the {owner}'s Session, `page.request` judges no Block cause and answers the 429"
        ),
        requested["result"]["status"] == json!(429),
    );
    let article = url_on(site, CORPUS_TASK, "/corpus/article").unwrap_or_default();
    let went = goto(connection, &page, &article)?;
    checks.check(
        format!("the {owner}'s page then loads an article"),
        answered(&went, 200),
    );
    finish_run(connection, &run_id)?;
    checks.check(
        format!("the {owner}'s Run kept one Session: no block replaced it"),
        kept_one_session(target, &run_id),
    );
    Ok(())
}

/// The access-policy shelves whose second page answers `402`, `403` and a
/// challenge served with `200`, walked over plain HTTP by a program's
/// Session from the first page to the third: whatever the second page
/// raises or answers (its Block cause is the other cases'), the third loads
/// on the same Session at once, the Run keeping that one Session,
/// with nothing written to Politeness's domain limits (#711's third
/// criterion).
fn access_policy(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-blocked-access-policy";
    let shelves = ["access-policy/7", "access-policy/8", "access-policy/9"];
    if shelves.iter().any(|task| site.start_url(task).is_none()) {
        checks.check("the Proving ground serves the access-policy shelves", false);
        return Ok(());
    }
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    for task in shelves {
        let mut statuses = Vec::new();
        for number in 1..=3 {
            let host = format!("{}.access-policy.test", &task["access-policy/".len()..]);
            let url = site
                .named_url(&host, &format!("/catalog/{number}"))
                .unwrap_or_default();
            let went = goto(connection, &page, &url)?;
            statuses.push(went["result"]["response"]["status"].clone());
        }
        checks.check(
            format!(
                "`{task}`'s first page answers, and its third on the same page after its second"
            ),
            statuses.first() == Some(&json!(200)) && statuses.last() == Some(&json!(200)),
        );
    }
    finish_run(connection, run_id)?;
    checks.check(
        "the Run kept one Session: no second one was opened",
        kept_one_session(target, run_id),
    );
    checks.check(
        "nothing in the Politeness file names the access-policy domain",
        !politeness_names(target, "access-policy.test"),
    );
    Ok(())
}
