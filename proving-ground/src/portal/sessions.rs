//! The portal's session tasks, as the lab's B2B portal served them: five
//! pages of twenty reports behind a login form with a CSRF token, a session
//! cookie that a variant expires after a few pages, a refresh token, and
//! revocation while a crawl is stopped. Each sign-in is an Effect the
//! Proving ground records, so a crawl that signs in more than it needs to is
//! judged from the site's own record.

use std::collections::{BTreeMap, BTreeSet};

use hyper::{Method, StatusCode};

use super::{Item, Spec, account, expected, page, push, report, token};
use crate::exchange::{self, Reply, Request, form_value};
use crate::task::Effect;

const PAGES: u64 = 5;
const REPORTS_PER_PAGE: u64 = 20;

/// The sessions of every session task, by ordinal.
#[derive(Default)]
pub(super) struct Sessions(BTreeMap<u32, TaskSessions>);

/// One task's sessions.
#[derive(Default)]
struct TaskSessions {
    /// How many tokens the task handed out.
    tokens: u64,
    /// The CSRF token of each login form served, by its `pre` cookie.
    pending: BTreeMap<String, String>,
    /// How many report pages each live session served, by its `sid`.
    live: BTreeMap<String, u64>,
    refresh: BTreeSet<String>,
}

impl TaskSessions {
    fn token(&mut self, seed: u64, ordinal: u32) -> String {
        self.tokens += 1;
        token(seed, ordinal, self.tokens)
    }
}

/// A session task's variant.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Session {
    /// A CSRF login in front of the reports.
    LoginCsrf,
    /// A session that serves four pages, then answers with the login form
    /// and a `200`, as many portals do.
    Expiry,
    /// A session that serves three pages, renewed with a refresh token.
    Refresh,
    /// A crawl stopped after two pages and resumed with its session.
    Resume,
    /// A crawl stopped after two pages, whose session the harness revokes
    /// before it resumes.
    ResumeRevoked,
}

impl Session {
    /// How many report pages one session serves, if it expires.
    fn pages(self) -> Option<u64> {
        match self {
            Self::Expiry => Some(4),
            Self::Refresh => Some(3),
            Self::LoginCsrf | Self::Resume | Self::ResumeRevoked => None,
        }
    }
}

/// The session task of `variant`.
pub(super) fn task(seed: u64, variant: Session) -> Spec {
    let (description, sign_ins) = match variant {
        Session::LoginCsrf => (
            "Sign in with the task's account through the login form, whose CSRF token \
             the form carries, then crawl the reports from the first page through each \
             Next link, and return one Record per report with the fields `id` and \
             `price`.",
            1,
        ),
        Session::Expiry => (
            "Sign in with the task's account, then crawl the reports from the first page \
             through each Next link and return one Record per report with the fields `id` \
             and `price`. A session serves four pages; an expired one is answered with \
             the login form and a 200, so sign in again only then.",
            2,
        ),
        Session::Refresh => (
            "Sign in with the task's account, then crawl the reports from the first page \
             through each Next link and return one Record per report with the fields `id` \
             and `price`. A session serves three pages; renew it by posting to \
             `/session/refresh` with the refresh token cookie `rt`, never by signing in \
             again.",
            1,
        ),
        Session::Resume => (
            "Sign in with the task's account and crawl the reports from the first page \
             through each Next link, returning one Record per report with the fields `id` \
             and `price`. The harness stops the tool after two pages and resumes it in a \
             new process, which reuses the journaled session rather than signing in \
             again.",
            1,
        ),
        Session::ResumeRevoked => (
            "Sign in with the task's account and crawl the reports from the first page \
             through each Next link, returning one Record per report with the fields `id` \
             and `price`. The harness stops the tool after two pages, revokes every \
             session (`POST /revoke-sessions` on the Proving ground's own address) and \
             resumes it in a new process, which must sign in again.",
            2,
        ),
    };
    Spec {
        start: "/reports?page=1",
        description,
        expected: expected(
            (1..=PAGES * REPORTS_PER_PAGE).map(|n| report(seed, n)),
            sign_ins,
        ),
        signs_in: true,
    }
}

impl Sessions {
    /// Revokes every live session and refresh token.
    pub(super) fn revoke(&mut self) {
        for task in self.0.values_mut() {
            task.live.clear();
            task.refresh.clear();
        }
    }

    /// Answers a request to the session task `ordinal`.
    pub(super) fn respond(
        &mut self,
        seed: u64,
        ordinal: u32,
        variant: Session,
        request: &Request<'_>,
    ) -> (Reply, Option<Effect>) {
        let task = self.0.entry(ordinal).or_default();
        match (request.method, request.path) {
            (&Method::GET, "/login") => (login_page(seed, ordinal, task, StatusCode::OK), None),
            (&Method::POST, "/login") => sign_in(seed, ordinal, variant, task, request),
            (&Method::POST, "/session/refresh") => (refresh(seed, ordinal, task, request), None),
            (&Method::GET, "/reports") => (reports(seed, ordinal, variant, task, request), None),
            (&Method::GET, "/") => (Reply::found("/reports?page=1"), None),
            _ => (Reply::not_found(), None),
        }
    }
}

/// The login form, with a new CSRF token tied to a new `pre` cookie.
fn login_page(seed: u64, ordinal: u32, task: &mut TaskSessions, status: StatusCode) -> Reply {
    let pre = task.token(seed, ordinal);
    let csrf = task.token(seed, ordinal);
    let form = format!(
        "<form id=\"login\" method=\"post\" action=\"/login\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><label>User <input name=\"user\"></label><label>Password <input name=\"password\" type=\"password\"></label><button type=\"submit\">Sign in</button></form>"
    );
    task.pending.insert(pre.clone(), csrf);
    Reply::html_with_status(status, page("Sign in", &form)).set_cookie("pre", &pre)
}

/// Signs in from the login form: the form's CSRF token must be the one its
/// `pre` cookie was given, then the account must match.
fn sign_in(
    seed: u64,
    ordinal: u32,
    variant: Session,
    task: &mut TaskSessions,
    request: &Request<'_>,
) -> (Reply, Option<Effect>) {
    let form = std::str::from_utf8(request.body).unwrap_or_default();
    let expected = request
        .cookie("pre")
        .and_then(|pre| task.pending.remove(pre));
    if expected.is_none() || form_value(form, "csrf") != expected {
        let refused = page("Forbidden", "<p>Invalid CSRF token</p>");
        return (
            Reply::html_with_status(StatusCode::FORBIDDEN, refused),
            None,
        );
    }
    let account = account(seed);
    if form_value(form, "user").as_deref() != Some(&account.username)
        || form_value(form, "password").as_deref() != Some(&account.password)
    {
        return (
            login_page(seed, ordinal, task, StatusCode::UNAUTHORIZED),
            None,
        );
    }
    let session = task.token(seed, ordinal);
    task.live.insert(session.clone(), 0);
    let next = request
        .parameter("next")
        .filter(|next| next.starts_with('/') && !next.starts_with("//"))
        .unwrap_or_else(|| "/reports?page=1".to_owned());
    let mut reply = Reply::see_other(&next).set_cookie("sid", &session);
    if variant == Session::Refresh {
        let refresh = task.token(seed, ordinal);
        task.refresh.insert(refresh.clone());
        reply = with_refresh_token(reply, &refresh);
    }
    (reply, Some(Effect::SignIn))
}

/// Adds the refresh token's cookie, sent only to `/session`.
fn with_refresh_token(mut reply: Reply, refresh: &str) -> Reply {
    reply.headers.push((
        "set-cookie",
        format!("rt={refresh}; Path=/session; HttpOnly; SameSite=Strict"),
    ));
    reply
}

/// Renews a session from its refresh token, which is used up.
fn refresh(seed: u64, ordinal: u32, task: &mut TaskSessions, request: &Request<'_>) -> Reply {
    if !request
        .cookie("rt")
        .is_some_and(|refresh| task.refresh.remove(refresh))
    {
        let refused = page("Unauthorized", "<p>Refresh token invalid</p>");
        return Reply::html_with_status(StatusCode::UNAUTHORIZED, refused);
    }
    let session = task.token(seed, ordinal);
    let refresh = task.token(seed, ordinal);
    task.live.insert(session.clone(), 0);
    task.refresh.insert(refresh.clone());
    with_refresh_token(Reply::no_content().set_cookie("sid", &session), &refresh)
}

/// A page of reports for a live session; a request without one goes to the
/// login form, and an expired one is answered as the variant says.
fn reports(
    seed: u64,
    ordinal: u32,
    variant: Session,
    task: &mut TaskSessions,
    request: &Request<'_>,
) -> Reply {
    let Some(number) = request
        .parameter("page")
        .and_then(|page| page.parse::<u64>().ok())
        .filter(|page| (1..=PAGES).contains(page))
    else {
        return Reply::not_found();
    };
    let back = format!(
        "/login?next={}",
        exchange::encode(&format!("{}?{}", request.path, request.query))
    );
    let Some(session) = request.cookie("sid").map(str::to_owned) else {
        return Reply::found(&back);
    };
    let Some(served) = task.live.get_mut(&session) else {
        return Reply::found(&back);
    };
    if variant.pages().is_some_and(|pages| *served >= pages) {
        task.live.remove(&session);
        return if variant == Session::Expiry {
            login_page(seed, ordinal, task, StatusCode::OK)
        } else {
            Reply::found(&back)
        };
    }
    *served += 1;
    let username = account(seed).username;
    let mut body = format!("<main id=\"records\" data-auth=\"{username}\" data-page=\"{number}\">");
    for n in (number - 1) * REPORTS_PER_PAGE + 1..=number * REPORTS_PER_PAGE {
        let item: Item = report(seed, n);
        body.push_str(&item.article(&item.id, item.price));
    }
    body.push_str("</main>");
    if number < PAGES {
        push(
            &mut body,
            format_args!(
                "<a class=\"next\" href=\"/reports?page={}\">Next</a>",
                number + 1
            ),
        );
    }
    Reply::html(page(&format!("Reports {number}"), &body))
}
