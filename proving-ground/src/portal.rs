//! The `portal` family, ported from the lab's portal site with its oracle:
//! each task on its own host name, `<ordinal>.portal.test`, serves one of the
//! lab's portal variants. Session tasks hold reports behind a CSRF login
//! whose sessions expire, refresh and are revoked, and their expected result
//! names each sign-in the crawl needs. Routing tasks hold one listing behind
//! six kinds of rendering, and frontier tasks a crawler trap and a table
//! that changes while it is paged. Pages and expected results come from the
//! seed; the state the family keeps is its sessions and the table.

mod frontier;
mod listing;
mod sessions;

use std::fmt::Write as _;

use listing::Rendering;
use sessions::Session;

use crate::exchange::{Reply, Request};
use crate::generator;
use crate::sync::Mutex;
use crate::task::{Account, Effect, Entry, Expected, Kind, Record, Task};

/// The host name the family's task host names end with.
pub(crate) const HOST: &str = "portal.test";

const FAMILY: &str = "portal";

/// The generator places of the family's values: the family's own first,
/// then the value's, then the task's ordinal or the Record's number where a
/// value has several.
const FAMILY_PLACE: u64 = 11;
const USERNAME_PLACE: u64 = 1;
const PASSWORD_PLACE: u64 = 2;
const TOKEN_PLACE: u64 = 3;
const REPORT_PLACE: u64 = 4;

const PASSWORD_LENGTH: u64 = 16;
const PASSWORD_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";

/// One task's variant of the portal.
#[derive(Clone, Copy)]
enum Variant {
    Session(Session),
    Listing(Rendering),
    /// A crawler trap.
    Trap,
    /// A table that changes while it is paged.
    Table,
}

/// The variants in ordinal order, from 1.
const VARIANTS: [Variant; 13] = [
    Variant::Session(Session::LoginCsrf),
    Variant::Session(Session::Expiry),
    Variant::Session(Session::Refresh),
    Variant::Session(Session::Resume),
    Variant::Session(Session::ResumeRevoked),
    Variant::Listing(Rendering::Static),
    Variant::Listing(Rendering::JsShell),
    Variant::Listing(Rendering::PartialHydration),
    Variant::Listing(Rendering::StaleHtml),
    Variant::Listing(Rendering::Graphql),
    Variant::Listing(Rendering::InfiniteScroll),
    Variant::Trap,
    Variant::Table,
];

/// The ordinal of the changing table's task.
const TABLE_ORDINAL: usize = 13;

/// The state the family keeps across requests.
pub(crate) struct Portal {
    seed: u64,
    sessions: Mutex<sessions::Sessions>,
    table: Mutex<frontier::Table>,
}

impl Portal {
    pub(crate) fn new(seed: u64) -> Self {
        Self {
            seed,
            sessions: Mutex::default(),
            table: Mutex::new(frontier::Table::new(seed)),
        }
    }

    /// Answers a request to a task host name, `name` without its port, if it
    /// is one of the family's: the reply, and the Effect it carried out.
    pub(crate) fn respond(
        &self,
        name: &str,
        request: &Request<'_>,
    ) -> Option<(Reply, Option<Effect>)> {
        let ordinal = ordinal(name)?;
        let variant = *VARIANTS.get(usize::try_from(ordinal).ok()?.checked_sub(1)?)?;
        Some(match variant {
            Variant::Session(session) => self
                .sessions
                .lock()
                .respond(self.seed, ordinal, session, request),
            Variant::Listing(rendering) => (listing::respond(self.seed, rendering, request), None),
            Variant::Trap => (frontier::trap(self.seed, request), None),
            Variant::Table => (self.table.lock().respond(request), None),
        })
    }

    /// The expected result of `task` as the family's state now makes it,
    /// for a task whose expected result changes while it runs: the changing
    /// table's.
    pub(crate) fn expected_now(&self, task: &str) -> Option<Expected> {
        (task == format!("{FAMILY}/{TABLE_ORDINAL}")).then(|| self.table.lock().expected())
    }

    /// Revokes every live session and refresh token of the family, as when a
    /// portal revokes them while a crawl is stopped.
    pub(crate) fn revoke(&self) {
        self.sessions.lock().revoke();
    }
}

/// The ordinal of a task host name, `<ordinal>.portal.test`.
fn ordinal(name: &str) -> Option<u32> {
    let ordinal = name.strip_suffix(HOST)?.strip_suffix('.')?;
    let number = ordinal.parse::<u32>().ok()?;
    (number.to_string() == ordinal).then_some(number)
}

fn task_host(ordinal: u32) -> String {
    format!("{ordinal}.{HOST}")
}

/// Every task host name of the family.
pub(crate) fn hosts() -> Vec<(String, String)> {
    (1..=ordinals())
        .map(|ordinal| (task_host(ordinal), format!("{FAMILY}/{ordinal}")))
        .collect()
}

fn ordinals() -> u32 {
    u32::try_from(VARIANTS.len()).expect("invariant: the family has few tasks")
}

/// The family's account, derived from the seed (ADR 0063).
fn account(seed: u64) -> Account {
    let alphabet =
        u64::try_from(PASSWORD_ALPHABET.len()).expect("invariant: the alphabet is short");
    Account {
        username: format!(
            "portal-{:08x}",
            generator::draw(seed, &[FAMILY_PLACE, USERNAME_PLACE]) >> 32
        ),
        password: (0..PASSWORD_LENGTH)
            .map(|index| {
                let drawn =
                    generator::below(seed, &[FAMILY_PLACE, PASSWORD_PLACE, index], alphabet);
                char::from(
                    PASSWORD_ALPHABET
                        [usize::try_from(drawn).expect("invariant: an index below 57")],
                )
            })
            .collect(),
        totp_secret: None,
    }
}

/// A token the family hands out: a session ID, a CSRF token or a refresh
/// token, the `count`th of task `ordinal`.
fn token(seed: u64, ordinal: u32, count: u64) -> String {
    format!(
        "{:016x}",
        generator::draw(
            seed,
            &[FAMILY_PLACE, TOKEN_PLACE, u64::from(ordinal), count]
        )
    )
}

/// An item a portal page shows, the family's Record: its ID and price.
#[derive(Clone)]
struct Item {
    id: String,
    price: u64,
}

fn report(seed: u64, number: u64) -> Item {
    Item {
        id: format!("r{number:03}"),
        price: 100 + generator::below(seed, &[FAMILY_PLACE, REPORT_PLACE, number], 900),
    }
}

impl Item {
    fn record(&self) -> Record {
        Record::from([("id", self.id.clone()), ("price", self.price.to_string())])
    }

    /// The item as the pages show it, with `price` in place of its own.
    fn article(&self, name: &str, price: u64) -> String {
        format!(
            "<article data-id=\"{}\" data-price=\"{price}\"><h2>{name}</h2></article>",
            self.id
        )
    }
}

/// A whole page.
fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head><body>{body}</body></html>"
    )
}

/// What one task's variant gives its task list entry.
struct Spec {
    /// The start page's path and query.
    start: &'static str,
    description: &'static str,
    expected: Expected,
    /// Whether the task signs in with the family's account.
    signs_in: bool,
}

/// The family's tasks for `seed`, their start URLs on their own host names
/// and `port`. Ordinals are append-only (ADR 0063).
pub(crate) fn tasks(seed: u64, port: u16) -> Vec<Task> {
    (1..=ordinals())
        .zip(VARIANTS)
        .map(|(ordinal, variant)| {
            let spec = match variant {
                Variant::Session(session) => sessions::task(seed, session),
                Variant::Listing(rendering) => listing::task(seed, rendering),
                Variant::Trap => frontier::trap_task(seed),
                Variant::Table => frontier::table_task(seed),
            };
            Task {
                host: task_host(ordinal),
                entry: Entry {
                    id: format!("{FAMILY}/{ordinal}"),
                    family: FAMILY,
                    kind: Kind::Crawl,
                    start_url: format!("http://{}:{port}{}", task_host(ordinal), spec.start),
                    description: spec.description,
                    account: spec.signs_in.then(|| account(seed)),
                    time_steps: None,
                },
                expected: spec.expected,
            }
        })
        .collect()
}

/// The expected Records of `items`, and `sign_ins` sign-ins.
fn expected(items: impl Iterator<Item = Item>, sign_ins: usize) -> Expected {
    Expected {
        records: items.map(|item| item.record()).collect(),
        effects: vec![Effect::SignIn; sign_ins],
        ..Expected::default()
    }
}

/// Appends a formatted piece to a page.
fn push(html: &mut String, piece: std::fmt::Arguments<'_>) {
    html.write_fmt(piece)
        .expect("invariant: writing to a String succeeds");
}
