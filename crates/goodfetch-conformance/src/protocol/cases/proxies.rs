//! The page suite's proxy cases (ADRs 0021, 0053, 0088 and 0092; ADR 0037 /
//! SA-22; pitfall H12, ticket #708), on the Proving ground's forward
//! proxies, whose connection record says which proxy each request went
//! through: a proxy that refuses the tunnel, one that is down and one that
//! asks for credentials the Session does not give each raise the proxy's
//! own Error code, never `navigation_failed`; two Sessions of one Run reach
//! the site each through its own proxy; a proxy on a page or a request is
//! refused; `proxy.server`'s URL takes neither `socks4` nor credentials;
//! a SOCKS5 proxy's credentials reach it through the Session's loopback
//! relay; and a Client acting for a model sets no proxy, on a Run, a
//! Session or the project configuration it sends.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::proxy::{PROXY_AUTH_FAILED, PROXY_UNAVAILABLE};
use goodfetch_protocol::page::proxy::{PROXY_PASSWORD, PROXY_SERVER};
use goodfetch_protocol::page::session::{RUN_NEW_SESSION, SESSION_NEW_PAGE};
use goodfetch_protocol::page::{PAGE_GOTO, PageOpened, RUN_NEW_PAGE};
use goodfetch_protocol::run::RUN_START;
use goodfetch_protocol::settings::{SETTING_INVALID, SETTING_LOCKED, SETTING_NOT_APPLICABLE};
use serde_json::{Value, json};

use super::{Checks, Site, finish_run, new_page, refused_as, start_run};
use crate::protocol::PROXY_PASSWORD_VARIABLE;
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::hello_as;
use crate::protocol::target::EngineTarget;

/// One proxy case of the page suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    Failing,
    OwnProxies,
    NonePerPage,
    ServerUrl,
    Socks5Relay,
    NoneByAModel,
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-failing-proxy-raises-its-own-error-never-navigation-failed" => Self::Failing,
            "two-sessions-of-a-run-reach-the-site-each-through-its-own-proxy" => Self::OwnProxies,
            "a-proxy-per-page-or-per-request-is-refused" => Self::NonePerPage,
            "socks4-and-credentials-in-the-proxy-url-are-setting-invalid" => Self::ServerUrl,
            "the-socks5-proxy-credentials-reach-it-through-the-sessions-relay" => Self::Socks5Relay,
            "a-client-acting-for-a-model-sets-no-proxy" => Self::NoneByAModel,
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
            Self::Failing => failing(connection, site, checks),
            Self::OwnProxies => own_proxies(connection, site, checks),
            Self::NonePerPage => none_per_page(connection, site, checks),
            Self::ServerUrl => server_url(connection, site, checks),
            Self::Socks5Relay => socks5_relay(connection, site, checks),
            Self::NoneByAModel => none_by_a_model(connection, target, site, checks),
        }
    }
}

/// `proxy.server` for the Proving ground's proxy `name`, checked to be one
/// it serves.
pub(super) fn server(site: &Site, name: &str, checks: &mut Checks) -> Option<String> {
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
        .replacen("http://", &format!("{scheme}://"), 1);
    let answer = match page {
        Some(page) => connection.call(
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
    site.observations()
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

/// A Session whose `proxy.server` is the Proving ground's `socks5` proxy,
/// with its username and the password the harness gave the target, loads
/// a page: Chromium gives a SOCKS5 proxy no credentials, so they reach it
/// through the Session's loopback relay, which the proxy's record shows
/// relaying with the Session's username (ADR 0037 / SA-22).
fn socks5_relay(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let address = site.proxy("socks5");
    let credentials = site.proxy_credentials();
    checks.check(
        "the Proving ground serves its `socks5` proxy and the credentials it takes",
        address.is_some() && credentials.is_some(),
    );
    let (Some(address), Some(credentials)) = (address, credentials) else {
        return Ok(());
    };
    let run_id = "conformance-proxy-socks5-relay";
    start_run(
        connection,
        site,
        run_id,
        &json!({ "proxy": {
            "server": format!("socks5://{address}"),
            "username": credentials.username,
            "password": format!("env:{PROXY_PASSWORD_VARIABLE}"),
        } }),
        checks,
    )?;
    let page = new_page(connection, run_id, checks)?;
    let url = site.named_url("scope.test", "/storage?mark=socks5-relay");
    let status = match page {
        Some(page) => connection.call(
            PAGE_GOTO.name,
            &json!({ "page_id": page.page_id, "url": url }),
        )?["result"]["response"]["status"]
            .clone(),
        None => Value::Null,
    };
    finish_run(connection, run_id)?;
    checks.check(
        "a page loads through the SOCKS5 proxy with credentials",
        status == json!(200),
    );
    let relayed = site
        .observations()
        .and_then(|observations| observations["relays"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .any(|relay| {
            relay["proxy"] == json!("socks5")
                && relay["username"] == json!(credentials.username)
                && relay["outcome"] == json!("relayed")
        });
    checks.check(
        "the SOCKS5 proxy relayed the Session's connection under its username",
        relayed,
    );
    Ok(())
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
    let opened = connection.call(
        RUN_NEW_SESSION.name,
        &json!({ "run_id": run_id, "settings": { "proxy": { "server": own } } }),
    )?;
    let own_page = connection.call(
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
            let url = site.named_url("scope.test", &format!("/storage?mark={mark}"));
            let answer = connection.call(
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
/// refused: a Session's proxy is the only one (SA-22). A page's own
/// `proxy.server` is a setting no page can use, so it is
/// `setting_not_applicable` naming the key (ADR 0083); a `proxy` argument
/// is no parameter of the call, so it is `invalid_params`.
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
            PAGE_GOTO.name,
            json!({ "page_id": "page-1", "url": "http://scope.test/", "proxy": proxy }),
        ),
    ] {
        let id = connection.request(method, &params)?;
        let answer = connection.answer(id)?.message;
        refused_as(&answer, ProtocolFailure::InvalidParams, checks);
    }
    let id = connection.request(
        RUN_NEW_PAGE.name,
        &json!({ "run_id": run_id, "settings": { "proxy": proxy } }),
    )?;
    let data = &connection.answer(id)?.message["error"]["data"];
    checks.check(
        "a page's own `proxy.server` is `setting_not_applicable` naming the key",
        data["code"] == json!(SETTING_NOT_APPLICABLE.name)
            && data["key"] == json!(PROXY_SERVER.name),
    );
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
            data["code"] == json!(SETTING_INVALID.name) && data["key"] == json!(PROXY_SERVER.name),
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

/// The Error code, key and layer of a refusal `answer` names.
fn locked(answer: &Value) -> (&Value, &Value, &Value) {
    let data = &answer["error"]["data"];
    (&data["code"], &data["key"], &data["layer"])
}

/// On a Connection of the MCP server, a Client acting for a model sets no
/// proxy: `proxy.server` on a Run and in the project configuration it
/// sends, and `proxy.password` on a Session, each fail with
/// `setting_locked` naming the key and its layer (ADR 0092).
fn none_by_a_model(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let mut agent = connection
        .another(target)
        .map_err(|error| Gone::Failed(error.to_string()))?;
    let opened = hello_as(&mut agent, Product::Collie, Surface::Mcp);
    checks.check(
        "an Agent's Client, through the MCP server, opens its own Connection",
        opened.is_ok(),
    );
    let ran = if opened.is_ok() {
        refused_to_a_model(&mut agent, site, checks)
    } else {
        Ok(())
    };
    connection.keep(agent.take_exchanged());
    agent.close();
    ran
}

/// The model's three refusals, on `agent`, its Connection; no proxy is
/// reached, since each is refused before its scope opens.
fn refused_to_a_model(
    agent: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let server = "http://127.0.0.1:3128";
    let code = json!(SETTING_LOCKED.name);
    let on_the_run = agent.call(RUN_START.name,
        &json!({ "run_id": "conformance-proxy-model", "settings": { "proxy": { "server": server } } }),
    )?;
    checks.check(
        "a model's `proxy.server` on a Run fails with `setting_locked` on the run layer",
        locked(&on_the_run) == (&code, &json!(PROXY_SERVER.name), &json!("run")),
    );
    let in_the_project = agent.call(
        RUN_START.name,
        &json!({
            "run_id": "conformance-proxy-model-project",
            "project": {
                "path": "goodfetch.toml",
                "settings": { "proxy": { "server": server } },
            },
        }),
    )?;
    checks.check(
        "a model's `proxy.server` in the project configuration it sends fails with `setting_locked`",
        locked(&in_the_project)
            == (
                &code,
                &json!(PROXY_SERVER.name),
                &json!("project_configuration"),
            ),
    );
    start_run(
        agent,
        site,
        "conformance-proxy-model-session",
        &json!({}),
        checks,
    )?;
    let on_a_session = agent.call(
        RUN_NEW_SESSION.name,
        &json!({
            "run_id": "conformance-proxy-model-session",
            "settings": { "proxy": { "password": "env:GOODFETCH_CONFORMANCE_PROXY_PASSWORD" } },
        }),
    )?;
    checks.check(
        "a model's `proxy.password` on a Session fails with `setting_locked`",
        locked(&on_a_session) == (&code, &json!(PROXY_PASSWORD.name), &json!("run")),
    );
    finish_run(agent, "conformance-proxy-model-session")
}
