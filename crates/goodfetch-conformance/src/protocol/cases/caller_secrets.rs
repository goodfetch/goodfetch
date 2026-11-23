//! The page suite's cases of the caller as a Secret provider (ADR 0092,
//! ticket #652): a Session whose proxy password is a `caller:` reference
//! has it resolved at its use by `secrets.resolve`, sent to the Client of
//! the Connection that opened the Run with the reference's name, the Run
//! and the proxy's host, and loads a page through the proxy with the
//! value it answers; a failure reply, and no answer within
//! `timeout.secret`, which the engine cancels, each fail the use with
//! `secret_unavailable`, `failure: "resolver_failed"`, whose fix names
//! `resolveSecret`. That another Connection is never asked needs two
//! Connections to one engine, a Daemon's, which the engine's own tests
//! prove.

use goodfetch_protocol::connection::CANCEL;
use goodfetch_protocol::page::{PAGE_GOTO, RUN_NEW_PAGE};
use goodfetch_protocol::secrets::{SECRET_UNAVAILABLE, SECRETS_RESOLVE};
use serde_json::{Value, json};

use super::proxies::server;
use super::{Checks, Site, finish_run, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::BROWSER_DEADLINE;

/// The name the cases' `caller:` reference gives the program's resolver.
const NAME: &str = "proving_ground_proxy";

/// The Proving ground's proxy that asks for Basic credentials.
const PROXY: &str = "datacenter";

/// One case of the caller as a Secret provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    Answered,
    NotAnswered,
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-caller-reference-is-answered-by-the-runs-own-client-at-its-use" => Self::Answered,
            "a-caller-reference-its-client-fails-or-leaves-unanswered-is-resolver-failed" => {
                Self::NotAnswered
            }
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
            Self::Answered => answered(connection, site, checks),
            Self::NotAnswered => not_answered(connection, site, checks),
        }
    }
}

/// How the case's Client answers a `secrets.resolve`.
#[derive(Clone, Copy)]
enum Resolver<'a> {
    /// With this value.
    Value(&'a str),
    /// With a failure, as a resolver that threw.
    Fails,
    /// Not at all.
    Silent,
}

/// What the engine sent while a `run.new_page` of the Run was open: the
/// call's answer, then each `secrets.resolve` request and each `$/cancel`,
/// in arrival order.
struct Opened {
    answer: Value,
    asked: Vec<Value>,
    cancelled: Vec<Value>,
}

/// Starts the Run `run_id` with `settings`, opens a page of it, its
/// `secrets.resolve` answered as `resolver` says, and loads the storage
/// page through it when it opened, as `mark`: what the engine sent while
/// the page opened, and the load's status.
fn opened_through(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    settings: &Value,
    resolver: Resolver<'_>,
    checks: &mut Checks,
) -> Result<(Opened, Value), Gone> {
    start_run(connection, site, run_id, settings, checks)?;
    let id = connection.request(RUN_NEW_PAGE.name, &json!({ "run_id": run_id }))?;
    let mut opened = Opened {
        answer: Value::Null,
        asked: Vec::new(),
        cancelled: Vec::new(),
    };
    loop {
        let message = connection.next_within(BROWSER_DEADLINE)?;
        if message.get("method").is_none() && message["id"] == json!(id) {
            opened.answer = message;
            break;
        }
        if message["method"] == json!(SECRETS_RESOLVE.name) {
            match resolver {
                Resolver::Value(value) => connection.reply_secret(&message["id"], value)?,
                Resolver::Fails => connection.fail(
                    &message["id"],
                    "handler_failed",
                    "the program's resolver threw",
                )?,
                Resolver::Silent => {}
            }
            opened.asked.push(message);
        } else if message["method"] == json!(CANCEL) {
            opened.cancelled.push(message);
        }
    }
    let status = match opened.answer["result"]["page_id"].as_str() {
        Some(page_id) => {
            let url = site.named_url("scope.test", &format!("/storage?mark={run_id}"));
            connection.call(PAGE_GOTO.name, &json!({ "page_id": page_id, "url": url }))?
                ["result"]["response"]["status"]
                .clone()
        }
        None => Value::Null,
    };
    finish_run(connection, run_id)?;
    Ok((opened, status))
}

/// The proxy settings of a Run through the Proving ground's `datacenter`
/// proxy, its password the `caller:` reference, beside `more`; none when
/// the Proving ground serves no such proxy or no credentials.
fn through_proxy(site: &Site, more: &Value, checks: &mut Checks) -> Option<(Value, String)> {
    let server = server(site, PROXY, checks)?;
    let credentials = site.proxy_credentials();
    checks.check(
        "the Proving ground gives the credentials its proxies take",
        credentials.is_some(),
    );
    let credentials = credentials?;
    let host = server
        .trim_start_matches("http://")
        .rsplit_once(':')
        .map_or_else(String::new, |(host, _)| host.to_owned());
    let mut settings = json!({ "proxy": {
        "server": server,
        "username": credentials.username,
        "password": format!("caller:{NAME}"),
    } });
    if let (Some(settings), Some(more)) = (settings.as_object_mut(), more.as_object()) {
        settings.extend(more.clone());
    }
    Some((settings, host))
}

/// The Run's own Client answers its `caller:` proxy password when the
/// Session opens, asked once with the reference's name, the Run and the
/// proxy's host, and the page loads through the proxy with that value.
fn answered(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let Some((settings, host)) = through_proxy(site, &json!({}), checks) else {
        return Ok(());
    };
    let Some(password) = site
        .proxy_credentials()
        .map(|credentials| credentials.password.clone())
    else {
        return Ok(());
    };
    let run_id = "conformance-caller-answered";
    let (opened, status) = opened_through(
        connection,
        site,
        run_id,
        &settings,
        Resolver::Value(&password),
        checks,
    )?;
    let asked: Vec<&Value> = opened
        .asked
        .iter()
        .map(|request| &request["params"])
        .collect();
    checks.check(
        format!(
            "`{}` asks the Run's own Client once, with the reference's name, the Run and the proxy's host",
            SECRETS_RESOLVE.name
        ),
        asked.len() == 1
            && asked.iter().all(|params| {
                params["name"] == json!(NAME)
                    && params["run_id"] == json!(run_id)
                    && params["domain"] == json!(host)
            }),
    );
    checks.check(
        "the page loads through the proxy with the value the Client answered",
        status == json!(200),
    );
    Ok(())
}

/// A failure reply, and no answer within `timeout.secret`, which the
/// engine cancels, each fail the page's opening with `secret_unavailable`,
/// `failure: "resolver_failed"`, whose fix names `resolveSecret`.
fn not_answered(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    for (run_id, resolver, more) in [
        ("conformance-caller-fails", Resolver::Fails, json!({})),
        (
            "conformance-caller-silent",
            Resolver::Silent,
            json!({ "timeout": { "secret": 2_000 } }),
        ),
    ] {
        let Some((settings, _)) = through_proxy(site, &more, checks) else {
            return Ok(());
        };
        let (opened, _) = opened_through(connection, site, run_id, &settings, resolver, checks)?;
        let data = &opened.answer["error"]["data"];
        checks.check(
            format!(
                "`{run_id}`: the Client is asked, and the page's opening raises `{}` with `resolver_failed`",
                SECRET_UNAVAILABLE.name
            ),
            opened.asked.len() == 1
                && data["code"] == json!(SECRET_UNAVAILABLE.name)
                && data["failure"] == json!("resolver_failed"),
        );
        checks.check(
            format!("`{run_id}`: the fix names `resolveSecret`"),
            data["fix"]
                .as_str()
                .is_some_and(|fix| fix.contains("resolveSecret")),
        );
        let cancelled = opened.asked.first().is_some_and(|request| {
            opened
                .cancelled
                .iter()
                .any(|cancel| cancel["params"]["id"] == request["id"])
        });
        checks.check(
            format!("`{run_id}`: a request left unanswered, and only it, is cancelled"),
            cancelled == matches!(resolver, Resolver::Silent),
        );
    }
    Ok(())
}
