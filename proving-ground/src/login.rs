//! The `login` family on `login.test` (ADR 0025's logins with MFA and logins
//! that expire): orders behind a login form, for accounts derived from the
//! seed so each Journey variant has its own (ADR 0063). Account 1 logs in
//! with its password and then a TOTP code (RFC 6238) from its secret;
//! account 2 with its password alone, and its login expires after three
//! pages, so a crawl must log in again to read every order. Account 3, the
//! seeded login, logs in with its password alone, and its Session is drawn
//! from the seed and the time step it was opened at, so it expires when the
//! time step changes, never with the wall clock, and `firefox-profile` writes
//! the same Session into a Firefox profile. The other accounts' sessions are
//! the only state the family keeps; pages and expected results come from the
//! seed.

pub(crate) mod canaries;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use hyper::{Method, StatusCode};

use crate::clock;
use crate::exchange::{Reply, Request};
use crate::generator;
use crate::sync::Mutex;
use crate::task::{self, Entry, Expected, Kind, Record, Task, origin};
use crate::totp;

/// The host name the family is served on.
pub(crate) const HOST: &str = "login.test";

const FAMILY: &str = "login";

/// The account that logs in with a TOTP code after its password.
const MFA_ACCOUNT: u64 = 1;
/// The account whose login expires after [`EXPIRING_PAGES`] pages.
const EXPIRING_ACCOUNT: u64 = 2;
/// The account whose Session is drawn from the seed and its time step.
const SEEDED_ACCOUNT: u64 = 3;
const ACCOUNTS: [u64; 3] = [MFA_ACCOUNT, EXPIRING_ACCOUNT, SEEDED_ACCOUNT];
/// The task ordinal of the seeded login, `login/3`.
const SEEDED_TASK: u32 = 3;
/// How many time steps the seeded login's task spans: its Session expires
/// once in it.
const SEEDED_TIME_STEPS: u64 = 2;
const ORDERS: u64 = 6;
/// How many pages behind the login an expiring account's session serves.
const EXPIRING_PAGES: u32 = 3;

/// The generator places of the family's values: the family's own first, then
/// the account's number, the order's (0 for the account's own values) and the
/// value's, with a character's or byte's index last where a value has several.
const FAMILY_PLACE: u64 = 3;
const USERNAME_PLACE: u64 = 1;
const PASSWORD_PLACE: u64 = 2;
const SECRET_PLACE: u64 = 3;
const PENDING_PLACE: u64 = 4;
const SESSION_PLACE: u64 = 5;
const ADJECTIVE_PLACE: u64 = 6;
const NOUN_PLACE: u64 = 7;
const TOTAL_PLACE: u64 = 8;
const SEEDED_SESSION_PLACE: u64 = 9;

const PASSWORD_LENGTH: u64 = 16;
const PASSWORD_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
/// RFC 4226's recommended length for an HMAC-SHA-1 key.
const SECRET_BYTES: u64 = 20;

const ADJECTIVES: [&str; 8] = [
    "Copper", "Denim", "Linen", "Maple", "Pewter", "Sage", "Slate", "Walnut",
];
const NOUNS: [&str; 8] = [
    "Teapot", "Lantern", "Satchel", "Blanket", "Skillet", "Vase", "Stool", "Tray",
];

/// One login account, derived from the seed.
struct Account {
    number: u64,
    username: String,
    password: String,
    /// The TOTP key of an account with MFA.
    totp_key: Option<Vec<u8>>,
}

fn draw(seed: u64, place: &[u64], bound: u64) -> usize {
    usize::try_from(generator::below(seed, place, bound))
        .expect("invariant: a drawn index is below a short list's length")
}

/// The account numbered `number` for `seed`.
fn account(seed: u64, number: u64) -> Account {
    let username = format!(
        "user-{:08x}",
        generator::draw(seed, &[FAMILY_PLACE, number, 0, USERNAME_PLACE]) >> 32
    );
    let alphabet =
        u64::try_from(PASSWORD_ALPHABET.len()).expect("invariant: the alphabet is short");
    let password = (0..PASSWORD_LENGTH)
        .map(|index| {
            let place = [FAMILY_PLACE, number, 0, PASSWORD_PLACE, index];
            char::from(PASSWORD_ALPHABET[draw(seed, &place, alphabet)])
        })
        .collect();
    let totp_key = (number == MFA_ACCOUNT).then(|| {
        (0..SECRET_BYTES)
            .map(|index| {
                let place = [FAMILY_PLACE, number, 0, SECRET_PLACE, index];
                u8::try_from(generator::below(seed, &place, 256))
                    .expect("invariant: a value below 256 is a byte")
            })
            .collect()
    });
    Account {
        number,
        username,
        password,
        totp_key,
    }
}

/// How many pages one session of an account serves, for an account whose
/// login expires.
fn pages_per_session(number: u64) -> Option<u32> {
    (number == EXPIRING_ACCOUNT).then_some(EXPIRING_PAGES)
}

/// The Session token of the seeded login opened at time step `step`: the
/// same for every Proving ground on `seed`, and accepted only at `step`.
fn seeded_token(seed: u64, step: u64) -> String {
    format!(
        "{:016x}",
        generator::draw(
            seed,
            &[FAMILY_PLACE, SEEDED_ACCOUNT, 0, SEEDED_SESSION_PLACE, step]
        )
    )
}

/// A Session cookie the family sets.
pub(crate) struct SessionCookie {
    /// The host name it is set on, with no leading dot: a host-only cookie.
    pub(crate) host: String,
    pub(crate) name: &'static str,
    pub(crate) value: String,
}

/// The seeded login's Session cookie at time step `step`, as its task's
/// login sets it.
pub(crate) fn seeded_session(seed: u64, step: u64) -> SessionCookie {
    SessionCookie {
        host: task_host(SEEDED_TASK),
        name: "session",
        value: seeded_token(seed, step),
    }
}

/// The cookie value of an account's login waiting for its TOTP code.
fn pending_token(seed: u64, number: u64) -> String {
    format!(
        "{:016x}",
        generator::draw(seed, &[FAMILY_PLACE, number, 0, PENDING_PLACE])
    )
}

/// One order of an account, as its page shows it.
struct Order {
    id: String,
    item: String,
    /// In dollars, with two decimals.
    total: String,
}

fn order(seed: u64, account: u64, number: u64) -> Order {
    let place = |value| [FAMILY_PLACE, account, number, value];
    let cents = 100 + generator::below(seed, &place(TOTAL_PLACE), 49_900);
    Order {
        id: order_id(number),
        item: format!(
            "{} {}",
            ADJECTIVES[draw(seed, &place(ADJECTIVE_PLACE), 8)],
            NOUNS[draw(seed, &place(NOUN_PLACE), 8)]
        ),
        total: format!("{}.{:02}", cents / 100, cents % 100),
    }
}

fn order_id(number: u64) -> String {
    format!("O-{number:02}")
}

/// The family's sessions: who each one logged in as, and how many pages it
/// has served.
#[derive(Default)]
pub(crate) struct Logins(Mutex<Sessions>);

#[derive(Default)]
struct Sessions {
    /// How many sessions have been opened, which makes each token new.
    opened: u64,
    open: BTreeMap<String, Session>,
}

struct Session {
    account: u64,
    pages: u32,
}

impl Logins {
    /// Opens a session for an account and returns its token.
    fn open(&self, seed: u64, account: u64) -> String {
        let mut sessions = self.0.lock();
        sessions.opened += 1;
        let token = format!(
            "{:016x}",
            generator::draw(
                seed,
                &[FAMILY_PLACE, account, 0, SESSION_PLACE, sessions.opened]
            )
        );
        sessions
            .open
            .insert(token.clone(), Session { account, pages: 0 });
        token
    }

    /// The account a session token is logged in as, counting one more page
    /// served; `None` once it has expired, which closes it.
    fn serve(&self, token: &str) -> Option<u64> {
        let mut sessions = self.0.lock();
        let session = sessions.open.get_mut(token)?;
        session.pages += 1;
        let account = session.account;
        if pages_per_session(account).is_some_and(|limit| session.pages > limit) {
            sessions.open.remove(token);
            return None;
        }
        Some(account)
    }
}

/// What a request on the family's host name is answered from: the seed, the
/// sessions and the Proving ground's time step.
pub(crate) struct Visit<'a> {
    pub(crate) seed: u64,
    pub(crate) logins: &'a Logins,
    pub(crate) time_step: u64,
}

/// Answers a request on the family's host name.
pub(crate) fn respond(visit: &Visit<'_>, request: &Request<'_>) -> Reply {
    let Visit { seed, logins, .. } = *visit;
    match (request.method, request.path) {
        (&Method::GET, "/login") => Reply::html(login_page(None)),
        (&Method::POST, "/login") => log_in(visit, request),
        (&Method::GET, "/mfa") => match pending_account(seed, request) {
            Some(_) => Reply::html(mfa_page(None)),
            None => Reply::see_other("/login"),
        },
        (&Method::POST, "/mfa") => check_code(seed, logins, request),
        (&Method::GET, path) => behind_the_login(visit, request, path),
        _ => Reply::not_found(),
    }
}

/// The value of the field `name` in a submitted form.
fn field(request: &Request<'_>, name: &str) -> Option<String> {
    form_urlencoded::parse(request.body)
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

fn log_in(visit: &Visit<'_>, request: &Request<'_>) -> Reply {
    let Visit {
        seed,
        logins,
        time_step,
    } = *visit;
    let (username, password) = (field(request, "username"), field(request, "password"));
    let account = ACCOUNTS
        .into_iter()
        .map(|number| account(seed, number))
        .find(|account| {
            username.as_deref() == Some(&account.username)
                && password.as_deref() == Some(&account.password)
        });
    match account {
        None => Reply::html_with_status(
            StatusCode::UNAUTHORIZED,
            login_page(Some("Wrong username or password.")),
        ),
        Some(account) if account.totp_key.is_some() => {
            Reply::see_other("/mfa").set_cookie("pending", &pending_token(seed, account.number))
        }
        Some(account) if account.number == SEEDED_ACCOUNT => {
            let SessionCookie { name, value, .. } = seeded_session(seed, time_step);
            Reply::see_other("/account").set_cookie(name, &value)
        }
        Some(account) => {
            Reply::see_other("/account").set_cookie("session", &logins.open(seed, account.number))
        }
    }
}

/// The account whose login waits for its TOTP code, from the request's
/// `pending` cookie.
fn pending_account(seed: u64, request: &Request<'_>) -> Option<Account> {
    let token = request.cookie("pending")?;
    ACCOUNTS
        .into_iter()
        .find(|number| pending_token(seed, *number) == token)
        .map(|number| account(seed, number))
}

fn check_code(seed: u64, logins: &Logins, request: &Request<'_>) -> Reply {
    let Some(account) = pending_account(seed, request) else {
        return Reply::see_other("/login");
    };
    let key = account
        .totp_key
        .expect("invariant: only an account with MFA has a pending login");
    let code = field(request, "code").unwrap_or_default();
    // The step before and after the current one are accepted too, for a code
    // typed just as its step ended and for clocks a little apart (RFC 6238,
    // section 5.2).
    let now = clock::unix_seconds();
    let accepted = [
        now.saturating_sub(totp::STEP_SECONDS),
        now,
        now + totp::STEP_SECONDS,
    ]
    .iter()
    .any(|at| totp::code(&key, *at) == code);
    if accepted {
        Reply::see_other("/account").set_cookie("session", &logins.open(seed, account.number))
    } else {
        Reply::html_with_status(StatusCode::UNAUTHORIZED, mfa_page(Some("Wrong code.")))
    }
}

/// The account page and the order pages, which only a session serves; a
/// request without one, or whose session expired, is sent to the login page.
fn behind_the_login(visit: &Visit<'_>, request: &Request<'_>, path: &str) -> Reply {
    let Visit {
        seed,
        logins,
        time_step,
    } = *visit;
    let order_number = match path {
        "/account" => None,
        _ => match path
            .strip_prefix("/account/orders/")
            .and_then(|id| (1..=ORDERS).find(|number| order_id(*number) == id))
        {
            Some(number) => Some(number),
            None => return Reply::not_found(),
        },
    };
    let Some(number) = request.cookie("session").and_then(|token| {
        (token == seeded_token(seed, time_step))
            .then_some(SEEDED_ACCOUNT)
            .or_else(|| logins.serve(token))
    }) else {
        return Reply::see_other("/login");
    };
    match order_number {
        None => Reply::html(account_page(&account(seed, number).username)),
        Some(order_number) => Reply::html(order_page(&order(seed, number, order_number))),
    }
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{title}</title></head>\n<body>\n<main>\n{body}</main>\n</body>\n</html>\n"
    )
}

fn error_line(error: Option<&str>) -> String {
    error.map_or_else(String::new, |error| {
        format!("<p class=\"error\" role=\"alert\">{error}</p>\n")
    })
}

fn login_page(error: Option<&str>) -> String {
    page(
        "Log in",
        &format!(
            "<h1>Log in</h1>\n{}<form method=\"post\" action=\"/login\">\n<label>Username <input name=\"username\" autocomplete=\"username\" required></label>\n<label>Password <input name=\"password\" type=\"password\" autocomplete=\"current-password\" required></label>\n<button type=\"submit\">Log in</button>\n</form>\n",
            error_line(error)
        ),
    )
}

fn mfa_page(error: Option<&str>) -> String {
    page(
        "Authentication code",
        &format!(
            "<h1>Authentication code</h1>\n{}<form method=\"post\" action=\"/mfa\">\n<label>Code from your authenticator app <input name=\"code\" inputmode=\"numeric\" autocomplete=\"one-time-code\" pattern=\"[0-9]{{6}}\" required></label>\n<button type=\"submit\">Verify</button>\n</form>\n",
            error_line(error)
        ),
    )
}

fn account_page(username: &str) -> String {
    let orders = (1..=ORDERS).fold(String::new(), |mut orders, number| {
        let id = order_id(number);
        writeln!(
            orders,
            "<li><a href=\"/account/orders/{id}\">Order {id}</a></li>"
        )
        .expect("invariant: writing to a String succeeds");
        orders
    });
    page(
        "Your orders",
        &format!("<h1>Orders of {username}</h1>\n<ul class=\"orders\">\n{orders}</ul>\n"),
    )
}

fn order_page(order: &Order) -> String {
    let Order { id, item, total } = order;
    page(
        &format!("Order {id}"),
        &format!(
            "<article class=\"order\" data-id=\"{id}\">\n<h1 class=\"item\">{item}</h1>\n<p>Total: <span class=\"total\" data-total=\"{total}\">${total}</span></p>\n</article>\n<a href=\"/account\">Back to your orders</a>\n"
        ),
    )
}

/// The account as the task list names it.
fn listed(account: &Account) -> task::Account {
    task::Account {
        username: account.username.clone(),
        password: account.password.clone(),
        totp_secret: account
            .totp_key
            .as_deref()
            .map(|key| data_encoding::BASE32_NOPAD.encode(key)),
    }
}

/// The host name of the family's task `ordinal`, on which the task's pages
/// are served: `1.login.test` for `login/1`.
fn task_host(ordinal: u32) -> String {
    format!("{ordinal}.{HOST}")
}

/// The family's tasks for `seed`, their start URLs on their own host names
/// and `port`. Ordinals are append-only (ADR 0063).
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let start_url = |ordinal| format!("{}/login", origin(&task_host(ordinal), port));
    let orders = |number| {
        Expected::records(
            (1..=ORDERS)
                .map(|order| record(seed, number, order))
                .collect(),
        )
    };
    vec![
        Task {
            host: task_host(1),
            entry: Entry {
                id: format!("{FAMILY}/1"),
                family: FAMILY,
                kind: Kind::Crawl,
                start_url: start_url(1),
                description: "Log in as the task's account with its password, then with the \
                    TOTP code of its secret (RFC 6238), and return one Record per order the \
                    account page lists, read from its order page, with the fields `id`, `item` \
                    and `total`.",
                account: Some(listed(&account(seed, MFA_ACCOUNT))),
                time_steps: None,
            },
            expected: orders(MFA_ACCOUNT),
        },
        Task {
            host: task_host(2),
            entry: Entry {
                id: format!("{FAMILY}/2"),
                family: FAMILY,
                kind: Kind::Crawl,
                start_url: start_url(2),
                description: "Log in as the task's account with its password, and return one \
                    Record per order the account page lists, read from its order page, with \
                    the fields `id`, `item` and `total`. The login expires after three pages: \
                    log in again whenever the site sends to its login page.",
                account: Some(listed(&account(seed, EXPIRING_ACCOUNT))),
                time_steps: None,
            },
            expected: orders(EXPIRING_ACCOUNT),
        },
        Task {
            host: task_host(SEEDED_TASK),
            entry: Entry {
                id: format!("{FAMILY}/{SEEDED_TASK}"),
                family: FAMILY,
                kind: Kind::Crawl,
                start_url: start_url(SEEDED_TASK),
                description: "Log in as the task's account with its password, or bring in its \
                    login from the Firefox profile `firefox-profile` writes, and return one \
                    Record per order the account page lists, read from its order page, with \
                    the fields `id`, `item` and `total`, while the Proving ground's time step \
                    goes from 0 to the last of the task's time steps. The login expires when \
                    the time step changes: log in again whenever the site sends to its login \
                    page.",
                account: Some(listed(&account(seed, SEEDED_ACCOUNT))),
                time_steps: Some(SEEDED_TIME_STEPS),
            },
            expected: orders(SEEDED_ACCOUNT),
        },
    ]
}

/// The expected Record of an order: its fields as its page shows them, the
/// total without its currency sign.
fn record(seed: u64, account: u64, number: u64) -> Record {
    let Order { id, item, total } = order(seed, account, number);
    Record::from([("id", id), ("item", item), ("total", total)])
}
