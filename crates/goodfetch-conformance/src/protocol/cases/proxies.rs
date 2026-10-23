//! The page suite's proxy cases (ADRs 0021, 0053, 0088 and 0092; ADR 0037 /
//! SA-22; pitfall H12, ticket #708), on the Proving ground's forward
//! proxies, whose connection record says which proxy each request went
//! through: a proxy that refuses the tunnel, one that is down and one that
//! asks for credentials the Session does not give each raise the proxy's
//! own Error code, never `navigation_failed`; two Sessions of one Run reach
//! the site each through its own proxy; a proxy on a page or a request is
//! refused; and `proxy.server`'s URL takes neither `socks4` nor
//! credentials.

use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::proxy::{PROXY_AUTH_FAILED, PROXY_UNAVAILABLE};
use goodfetch_protocol::page::session::{RUN_NEW_SESSION, SESSION_NEW_PAGE};
use goodfetch_protocol::page::{PAGE_GOTO, PageOpened, RUN_NEW_PAGE};
use goodfetch_protocol::run::RUN_START;
use goodfetch_protocol::settings::SETTING_INVALID;
use serde_json::{Value, json};

use super::{Checks, Site, finish_run, new_page, refused_as, start_run};
use crate::protocol::connection::{Connection, Gone};

/// One proxy case of the page suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    Failing,
    OwnProxies,
    NonePerPage,
    ServerUrl,
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-failing-proxy-raises-its-own-error-never-navigation-failed" => Self::Failing,
            "two-sessions-of-a-run-reach-the-site-each-through-its-own-proxy" => Self::OwnProxies,
            "a-proxy-per-page-or-per-request-is-refused" => Self::NonePerPage,
            "socks4-and-credentials-in-the-proxy-url-are-setting-invalid" => Self::ServerUrl,
            _ => return None,
        })
    }

    /// Runs the case on the open Connection against `site`.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::Failing => failing(connection, site, checks),
            Self::OwnProxies => own_proxies(connection, site, checks),
            Self::NonePerPage => none_per_page(connection, site, checks),
            Self::ServerUrl => server_url(connection, site, checks),
        }
    }
}

/// Calls `method` with `params` and answers the engine's whole answer,
/// waiting as long as a browser's launch may take.
fn call(connection: &mut Connection, method: &str, params: &Value) -> Result<Value, Gone> {
    let id = connection.request(method, params)?;
    Ok(connection.browser_answer(id)?.message)
}

/// `proxy.server` for the Proving ground's proxy `name`, checked to be one
/// it serves.
fn server(site: &Site, name: &str, checks: &mut Checks) -> Option<String> {
    let address = site.proxy(name);
    checks.check(
        format!("the Proving ground serves its `{name}` proxy"),
        address.is_some(),
    );
    address.map(|address| format!("http://{address}"))
}

/// A navigation through the proxy `name` to the scope family's storage page
/// on `scheme`, on the Run `run_id`'s default Session: its answer.
fn through(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    server: &str,
    scheme: &str,
    checks: &mut Checks,
) -> Result<Value, Gone> {
    start_run(
        connection,
        site,
        run_id,
        &json!({ "proxy": { "server": server } }),
        checks,
    )?;
    let page = new_page(connection, run_id, checks)?;
    let url = site
        .named_url("scope.test", "/storage?mark=failing")
        .unwrap_or_default()
        .replacen("http://", &format!("{scheme}://"), 1);
    let answer = match page {
        Some(page) => call(
            connection,
            PAGE_GOTO.name,
            &json!({ "page_id": page.page_id, "url": url }),
        )?,
        None => Value::Null,
    };
    finish_run(connection, run_id)?;
    Ok(answer)
}

/// A proxy that refuses the tunnel to an `https://` page, one that is down
/// and one that asks for credentials raise `proxy_unavailable`, retryable,
/// and `proxy_auth_failed`, each naming the proxy, never
/// `navigation_failed` (pitfall H12).
fn failing(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    for (name, scheme, code, retryable) in [
        ("refusing_tunnels", "https", PROXY_UNAVAILABLE.name, true),
        ("down", "http", PROXY_UNAVAILABLE.name, true),
        ("datacenter", "http", PROXY_AUTH_FAILED.name, false),
    ] {
        let Some(server) = server(site, name, checks) else {
            continue;
        };
        let run_id = format!("conformance-proxy-{}", name.replace('_', "-"));
        let answer = through(connection, site, &run_id, &server, scheme, checks)?;
        let data = &answer["error"]["data"];
        checks.check(
            format!(
                "a navigation through the `{name}` proxy raises `{code}`, never `navigation_failed`"
            ),
            data["code"] == json!(code),
        );
        checks.check(
            format!(
                "`{code}` from the `{name}` proxy names it, and says whether a retry may succeed"
            ),
            data["proxy"] == json!(server) && data["retryable"] == json!(retryable),
        );
    }
    Ok(())
}

/// The relays the Proving ground's proxies carried out to the storage page
/// with a mark, in arrival order: each one's proxy and mark.
fn relays(site: &Site) -> Vec<(Value, String)> {
    site.proving_ground
        .observations()
        .ok()
        .and_then(|observations| observations["relays"].as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|relay| relay["outcome"] == json!("relayed"))
        .filter_map(|relay| {
            let target = relay["target"].as_str()?;
            let mark = target.split_once("/storage?mark=")?.1.to_owned();
            Some((relay["proxy"].clone(), mark))
        })
        .collect()
}

/// The Run's default Session through `open` and a Session of its own
/// through `refusing_tunnels`, which relays plain HTTP, each reach the
/// storage page through its own proxy.
fn own_proxies(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let (Some(runs), Some(own)) = (
        server(site, "open", checks),
        server(site, "refusing_tunnels", checks),
    ) else {
        return Ok(());
    };
    let run_id = "conformance-proxy-own";
    start_run(
        connection,
        site,
        run_id,
        &json!({ "proxy": { "server": runs } }),
        checks,
    )?;
    let default_page = new_page(connection, run_id, checks)?;
    let opened = call(
        connection,
        RUN_NEW_SESSION.name,
        &json!({ "run_id": run_id, "settings": { "proxy": { "server": own } } }),
    )?;
    let own_page = call(
        connection,
        SESSION_NEW_PAGE.name,
        &json!({ "session_id": opened["result"]["session_id"] }),
    )?;
    let own_page = serde_json::from_value::<PageOpened>(own_page["result"].clone()).ok();
    checks.check(
        "a Session with its own `proxy.server` opens a page",
        own_page.is_some(),
    );
    if let (Some(default_page), Some(own_page)) = (default_page, own_page) {
        let mut statuses = Vec::new();
        for (page, mark) in [(&default_page, "runs"), (&own_page, "own")] {
            let url = site
                .named_url("scope.test", &format!("/storage?mark={mark}"))
                .unwrap_or_default();
            let answer = call(
                connection,
                PAGE_GOTO.name,
                &json!({ "page_id": page.page_id, "url": url }),
            )?;
            statuses.push(answer["result"]["response"]["status"].clone());
        }
        checks.check(
            "each Session's page loads the storage page",
            statuses == vec![json!(200), json!(200)],
        );
        checks.check(
            "the default Session's request went through the Run's proxy, the other Session's through its own",
            relays(site)
                == vec![
                    (json!("open"), "runs".to_owned()),
                    (json!("refusing_tunnels"), "own".to_owned()),
                ],
        );
    }
    finish_run(connection, run_id)
}

/// A proxy given to a page, to a page's settings or to a request is
/// refused: a Session's proxy is the only one (SA-22).
fn none_per_page(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-proxy-per-page";
    start_run(connection, site, run_id, &json!({}), checks)?;
    let proxy = json!({ "server": "http://127.0.0.1:3128" });
    for (method, params) in [
        (
            SESSION_NEW_PAGE.name,
            json!({ "session_id": format!("{run_id}.session-1"), "proxy": proxy }),
        ),
        (
            RUN_NEW_PAGE.name,
            json!({ "run_id": run_id, "settings": { "proxy": proxy } }),
        ),
        (
            PAGE_GOTO.name,
            json!({ "page_id": "page-1", "url": "http://scope.test/", "proxy": proxy }),
        ),
    ] {
        let id = connection.request(method, &params)?;
        let answer = connection.answer(id)?.message;
        refused_as(&answer, ProtocolFailure::InvalidParams, checks);
    }
    finish_run(connection, run_id)
}

/// `socks4` and credentials in `proxy.server`'s URL fail with
/// `setting_invalid`, whose reason names `proxy.username` and
/// `proxy.password`.
fn server_url(connection: &mut Connection, _site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    for server in [
        "socks4://127.0.0.1:1080",
        "http://user:secret@127.0.0.1:3128",
    ] {
        let id = connection.request(
            RUN_START.name,
            &json!({ "run_id": "conformance-proxy-url", "settings": { "proxy": { "server": server } } }),
        )?;
        let answer = connection.answer(id)?.message;
        let data = &answer["error"]["data"];
        checks.check(
            format!("`{server}` is `setting_invalid` for `proxy.server`"),
            data["code"] == json!(SETTING_INVALID.name) && data["key"] == json!("proxy.server"),
        );
        checks.check(
            format!("the refusal of `{server}` names `proxy.username` and `proxy.password`"),
            data["reason"].as_str().is_some_and(|reason| {
                reason.contains("proxy.username") && reason.contains("proxy.password")
            }),
        );
    }
    Ok(())
}
