//! The `protection` family on `protection.test`: local Challenges, each with
//! its expected result. The tier-move site (Foxhound's spec #600) answers
//! every page with a Challenge its script passes, so a browser clears it and
//! a plain HTTP client never does; its oracle records which tier cleared
//! each page and the fingerprint of every request, and refutes a Record of a
//! page no tier cleared. The local CAPTCHA holds an article behind a code a
//! person reads off an image. The Turnstile page holds one behind a token the
//! solver-service stand-ins (`solvers`) answer its widget's task with, and
//! the sensor page behind the sensor data they answer.

mod sensor;
mod turnstile;

pub(crate) use sensor::{Sensed, data as sensor_data, hash, script_hash};
pub(crate) use turnstile::token as turnstile_token;

use std::fmt::Write as _;

use hyper::{Method, StatusCode};
use serde::Serialize;

use crate::exchange::{self, Reply, Request};
use crate::generator;
use crate::observation::{self, Observations, Recorded, Report, Seen};
use crate::oracle::{Answer, Finding};
use crate::task::{self, Entry, Expected, Kind, Record, Task};

/// The host name the family is served on.
pub(crate) const HOST: &str = "protection.test";

const FAMILY: &str = "protection";

/// How many tasks the family has.
const TASKS: u32 = 4;

/// The tier-move site's task.
pub(crate) const TIER_MOVE_TASK: &str = "protection/1";

/// The tier-move site's index, which links every article.
const INDEX: &str = "/articles";

const ARTICLES: u64 = 5;

/// The local CAPTCHA's page, and the article behind it.
const CAPTCHA: &str = "/captcha";

/// The cookies that carry a passed Challenge and a solved CAPTCHA.
const CLEARANCE_COOKIE: &str = "clearance";
const CAPTCHA_COOKIE: &str = "captcha";

/// The generator places of the family's values: the family's own first, then
/// the article's number (0 for the family's own values) and the value's.
const FAMILY_PLACE: u64 = 3;
const ADJECTIVE_PLACE: u64 = 1;
const NOUN_PLACE: u64 = 2;
const NONCE_PLACE: u64 = 3;
const CAPTCHA_PLACE: u64 = 4;
const CAPTCHA_TOKEN_PLACE: u64 = 5;
const SITE_KEY_PLACE: u64 = 6;
const TURNSTILE_COOKIE_PLACE: u64 = 7;
const SENSOR_PLACE: u64 = 8;

/// The CAPTCHA's article, numbered after the tier-move site's.
const CAPTCHA_ARTICLE: u64 = ARTICLES + 1;

/// The Turnstile page's article, numbered after the CAPTCHA's.
const TURNSTILE_ARTICLE: u64 = ARTICLES + 2;

/// The sensor page's article, numbered after the Turnstile page's.
const SENSOR_ARTICLE: u64 = ARTICLES + 3;

const ADJECTIVES: [&str; 8] = [
    "Quiet", "Hidden", "Northern", "Patient", "Restless", "Silver", "Sunken", "Winding",
];
const NOUNS: [&str; 8] = [
    "Harbour",
    "Orchard",
    "Lighthouse",
    "Meadow",
    "Quarry",
    "Railway",
    "Valley",
    "Workshop",
];

/// The characters a CAPTCHA code is drawn from, none that reads as another.
const CODE_CHARACTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const CODE_LENGTH: u64 = 5;

/// The Challenge's script: it computes the clearance from the page's nonce,
/// sets it as a cookie and loads the page again.
const CHALLENGE_SCRIPT: &str = include_str!("../pages/protection/challenge.js");

/// What a cleared page's script posts once it ran, so the site knows a
/// browser, not a plain HTTP client, read the page.
const RAN_SCRIPT: &str = "<script>navigator.sendBeacon(\"/reports\", JSON.stringify({ page: location.pathname, check: \"ran\", observed: \"content\" }));</script>\n";

/// Every host name the family is served on, each with its task.
pub(crate) fn hosts() -> Vec<(String, Option<String>)> {
    task::family_hosts(FAMILY, HOST, TASKS)
}

fn index(value: u64) -> usize {
    usize::try_from(value).expect("invariant: a word index is below 32")
}

fn article_id(number: u64) -> String {
    format!("T-{number:04}")
}

/// The title of the article numbered `number` (from 1) for `seed`.
fn title(seed: u64, number: u64) -> String {
    let draw = |value, bound| generator::below(seed, &[FAMILY_PLACE, number, value], bound);
    let adjective = ADJECTIVES[index(draw(ADJECTIVE_PLACE, ADJECTIVES.len() as u64))];
    let noun = NOUNS[index(draw(NOUN_PLACE, NOUNS.len() as u64))];
    format!("The {adjective} {noun}")
}

/// The Challenge's nonce for `seed`.
fn nonce(seed: u64) -> String {
    format!(
        "{:016x}",
        generator::draw(seed, &[FAMILY_PLACE, 0, NONCE_PLACE])
    )
}

/// The clearance the Challenge's script computes from `nonce`: each
/// character folded into 7 as `hash * 31 + code`, kept to 32 bits, in
/// lowercase hex.
fn clearance(nonce: &str) -> String {
    let hash = nonce.bytes().fold(7_u64, |hash, byte| {
        (hash * 31 + u64::from(byte)) & 0xFFFF_FFFF
    });
    format!("{hash:x}")
}

/// The CAPTCHA's code for `seed`.
fn captcha_code(seed: u64) -> String {
    (0..CODE_LENGTH)
        .map(|place| {
            let at = generator::below(
                seed,
                &[FAMILY_PLACE, 0, CAPTCHA_PLACE, place],
                CODE_CHARACTERS.len() as u64,
            );
            char::from(CODE_CHARACTERS[index(at)])
        })
        .collect()
}

/// The cookie value a solved CAPTCHA sets for `seed`.
fn captcha_token(seed: u64) -> String {
    format!(
        "{:016x}",
        generator::draw(seed, &[FAMILY_PLACE, 0, CAPTCHA_TOKEN_PLACE])
    )
}

/// Answers a request to one of the family's host names: its page and what
/// the site records of it, if anything; `None` when `host` is not the
/// family's.
pub(crate) fn respond(
    seed: u64,
    host: &str,
    request: &Request<'_>,
) -> Option<(Reply, Option<Recorded>)> {
    if !task::serves(host, HOST, TASKS) {
        return None;
    }
    let cleared = request.cookie(CLEARANCE_COOKIE) == Some(clearance(&nonce(seed)).as_str());
    Some(match (request.method, request.path) {
        (&Method::GET, path) if path == INDEX || article_number(path).is_some() => {
            if cleared {
                (Reply::html(cleared_page(seed, path)), None)
            } else {
                (
                    Reply::html_with_status(StatusCode::FORBIDDEN, challenge_page(seed)),
                    None,
                )
            }
        }
        (&Method::GET, CAPTCHA) => {
            if request.cookie(CAPTCHA_COOKIE) == Some(captcha_token(seed).as_str()) {
                (
                    Reply::html(article_page(seed, CAPTCHA_ARTICLE, "the CAPTCHA")),
                    None,
                )
            } else {
                (Reply::html(captcha_page(seed)), None)
            }
        }
        (&Method::POST, CAPTCHA) => {
            let answer = std::str::from_utf8(request.body)
                .ok()
                .and_then(|form| exchange::form_value(form, "answer"));
            if answer.as_deref() == Some(captcha_code(seed).as_str()) {
                (
                    Reply::html(solved_page(CAPTCHA))
                        .set_cookie(CAPTCHA_COOKIE, &captcha_token(seed)),
                    None,
                )
            } else {
                (
                    Reply::html_with_status(StatusCode::FORBIDDEN, captcha_page(seed)),
                    None,
                )
            }
        }
        (&Method::GET, turnstile::PATH) => {
            if request.cookie(turnstile::COOKIE) == Some(turnstile::cookie(seed).as_str()) {
                (
                    Reply::html(article_page(seed, TURNSTILE_ARTICLE, "the widget")),
                    None,
                )
            } else {
                (Reply::html(turnstile::page(seed)), None)
            }
        }
        (&Method::POST, turnstile::PATH) => {
            let token = std::str::from_utf8(request.body)
                .ok()
                .and_then(|form| exchange::form_value(form, turnstile::FIELD));
            if token.is_some_and(|token| turnstile::accepts(seed, host, &token)) {
                (
                    Reply::html(solved_page(turnstile::PATH))
                        .set_cookie(turnstile::COOKIE, &turnstile::cookie(seed)),
                    None,
                )
            } else {
                (
                    Reply::html_with_status(StatusCode::FORBIDDEN, turnstile::page(seed)),
                    None,
                )
            }
        }
        (&Method::POST, "/reports") => match Report::parse(request.body) {
            Some(report) => (Reply::no_content(), Some(Recorded::Report(report))),
            None => (
                Reply::error(StatusCode::BAD_REQUEST, "malformed_report"),
                None,
            ),
        },
        _ => {
            let article = || article_page(seed, SENSOR_ARTICLE, "the sensor");
            (
                sensor::respond(seed, host, request, article).unwrap_or_else(Reply::not_found),
                None,
            )
        }
    })
}

/// The number of the article at `path` (`/articles/N`), if the site has it.
fn article_number(path: &str) -> Option<u64> {
    let number = path.strip_prefix("/articles/")?;
    let parsed = number.parse::<u64>().ok()?;
    (parsed.to_string() == number && (1..=ARTICLES).contains(&parsed)).then_some(parsed)
}

/// The Challenge every page answers a client without a clearance with.
fn challenge_page(seed: u64) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Just a moment...</title></head>\n<body>\n<main id=\"challenge\">\n<h1>Checking your browser before accessing this site.</h1>\n<p>This takes a moment and needs JavaScript.</p>\n</main>\n<script data-nonce=\"{}\">\n{CHALLENGE_SCRIPT}</script>\n</body>\n</html>\n",
        nonce(seed)
    )
}

/// A page past its Challenge: the index or an article.
fn cleared_page(seed: u64, path: &str) -> String {
    let (page_title, main) = if let Some(number) = article_number(path) {
        let title = title(seed, number);
        let main = format!(
            "<article class=\"article\" data-id=\"{}\">\n<h1 class=\"title\">{title}</h1>\n<p>An article of the protected site.</p>\n</article>\n<a href=\"{INDEX}\">Every article</a>\n",
            article_id(number)
        );
        (title, main)
    } else {
        let mut main = "<h1>Articles</h1>\n<ul class=\"articles\">\n".to_owned();
        for number in 1..=ARTICLES {
            writeln!(
                main,
                "<li><a href=\"/articles/{number}\">{}</a></li>",
                title(seed, number)
            )
            .expect("invariant: writing to a String succeeds");
        }
        main.push_str("</ul>\n");
        ("Articles".to_owned(), main)
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{page_title}</title></head>\n<body>\n<main>\n{main}</main>\n{RAN_SCRIPT}</body>\n</html>\n"
    )
}

/// The local CAPTCHA: its code drawn as an image, one rotated glyph per
/// character, and the form that answers it.
fn captcha_page(seed: u64) -> String {
    let mut glyphs = String::new();
    for (place, character) in (0_u64..).zip(captcha_code(seed).chars()) {
        let rotation = i64::try_from(generator::below(
            seed,
            &[FAMILY_PLACE, 0, CAPTCHA_PLACE, CODE_LENGTH + place],
            41,
        ))
        .expect("invariant: a rotation is below 41")
            - 20;
        let x = 14 + 26 * place;
        writeln!(
            glyphs,
            "<text class=\"glyph\" x=\"{x}\" y=\"34\" transform=\"rotate({rotation} {x} 26)\">{character}</text>"
        )
        .expect("invariant: writing to a String succeeds");
    }
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Prove you are a person</title></head>\n<body>\n<main id=\"captcha\">\n<h1>Prove you are a person</h1>\n<svg role=\"img\" aria-label=\"A code of five characters\" width=\"150\" height=\"48\" font-family=\"monospace\" font-size=\"26\">\n<path d=\"M0 30 C40 10, 80 50, 150 20\" stroke=\"gray\" fill=\"none\"/>\n{glyphs}</svg>\n<form method=\"post\" action=\"{CAPTCHA}\">\n<label>Type the characters you see <input name=\"answer\" autocomplete=\"off\"></label>\n<button type=\"submit\">Continue</button>\n</form>\n</main>\n</body>\n</html>\n"
    )
}

/// The page a solved CAPTCHA or widget at `path` answers with, beside its
/// cookie.
fn solved_page(path: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Thank you</title></head>\n<body>\n<main>\n<p>Thank you. <a href=\"{path}\">Continue to the article</a>.</p>\n</main>\n</body>\n</html>\n"
    )
}

/// The page of the article numbered `number`, which `barrier` held.
fn article_page(seed: u64, number: u64, barrier: &str) -> String {
    let title = title(seed, number);
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{title}</title></head>\n<body>\n<main>\n<article class=\"article\" data-id=\"{}\">\n<h1 class=\"title\">{title}</h1>\n<p>The article behind {barrier}.</p>\n</article>\n</main>\n</body>\n</html>\n",
        article_id(number)
    )
}

/// The family's tasks for `seed`, their start URLs on their own host names
/// and `port`. Ordinals are append-only (ADR 0063).
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let origin = |ordinal| task::origin(&task::task_host(ordinal, HOST), port);
    let record =
        |number| Record::from([("id", article_id(number)), ("title", title(seed, number))]);
    vec![
        Task {
            host: task::task_host(1, HOST),
            entry: Entry {
                account: None,
                time_steps: None,
                id: TIER_MOVE_TASK.to_owned(),
                family: FAMILY,
                kind: Kind::Crawl,
                start_url: format!("{}{INDEX}", origin(1)),
                description: "Crawl the articles this index links and return one Record per \
                    article with the fields `id` and `title`. Every page answers a client \
                    without a clearance with a Challenge its script passes in a browser.",
            },
            expected: Expected::records((1..=ARTICLES).map(record).collect()),
        },
        Task {
            host: task::task_host(2, HOST),
            entry: Entry {
                account: None,
                time_steps: None,
                id: format!("{FAMILY}/2"),
                family: FAMILY,
                kind: Kind::Extract,
                start_url: format!("{}{CAPTCHA}", origin(2)),
                description: "Extract the article behind this CAPTCHA as one Record with the \
                    fields `id` and `title`.",
            },
            expected: Expected::records(vec![record(CAPTCHA_ARTICLE)]),
        },
        Task {
            host: task::task_host(3, HOST),
            entry: Entry {
                account: None,
                time_steps: None,
                id: format!("{FAMILY}/3"),
                family: FAMILY,
                kind: Kind::Extract,
                start_url: format!("{}{}", origin(3), turnstile::PATH),
                description: "Extract the article behind this page's Turnstile widget as one \
                    Record with the fields `id` and `title`. The page takes the token a \
                    solver-service stand-in answers for its site key and URL.",
            },
            expected: Expected::records(vec![record(TURNSTILE_ARTICLE)]),
        },
        Task {
            host: task::task_host(4, HOST),
            entry: Entry {
                account: None,
                time_steps: None,
                id: format!("{FAMILY}/4"),
                family: FAMILY,
                kind: Kind::Extract,
                start_url: format!("{}{}", origin(4), sensor::PATH),
                description: "Extract the article behind this page's sensor check as one \
                    Record with the fields `id` and `title`. The page takes the sensor data a \
                    solver-service stand-in answers for its URL, cookies, script and client.",
            },
            expected: Expected::records(vec![record(SENSOR_ARTICLE)]),
        },
    ]
}

/// Which tier cleared a page of the tier-move site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Tier {
    /// A browser: the page was served past its Challenge and its script ran.
    Browser,
    /// An HTTP client holding a clearance: the page was served past its
    /// Challenge and its script never ran.
    Http,
}

/// A page of the tier-move site and the tier that cleared it, `null` when
/// no request was served the page past its Challenge.
#[derive(Serialize)]
struct Cleared {
    page: String,
    cleared_by: Option<Tier>,
}

/// The oracle's answer for the tier-move site: the answer, the tier that
/// cleared each page and what the Proving ground saw of every request.
#[derive(Serialize)]
pub(crate) struct TierMoveAnswer {
    #[serde(flatten)]
    answer: Answer,
    pages: Vec<Cleared>,
    requests: Vec<Seen>,
}

/// Judges the tier-move site's `answer` from the Proving ground's own record:
/// each page's tier, and a finding for each submitted Record, `submitted` by
/// its ID, of a page no tier cleared.
pub(crate) fn judge_tier_move(
    answer: Answer,
    submitted: &[String],
    observations: &Observations,
) -> TierMoveAnswer {
    let requests = observations.requests_of(TIER_MOVE_TASK);
    let ran = observations.reports_of(TIER_MOVE_TASK);
    let pages: Vec<Cleared> = std::iter::once(INDEX.to_owned())
        .chain((1..=ARTICLES).map(|number| format!("/articles/{number}")))
        .map(|page| {
            let served = requests
                .iter()
                .any(|seen| seen.target == page && seen.status == Some(200));
            let script_ran = ran
                .iter()
                .any(|report| report.check == observation::RAN && report.page == page);
            let cleared_by = match (served, script_ran) {
                (false, _) => None,
                (true, true) => Some(Tier::Browser),
                (true, false) => Some(Tier::Http),
            };
            Cleared { page, cleared_by }
        })
        .collect();
    let findings = submitted
        .iter()
        .filter_map(|id| {
            let number = id.strip_prefix("T-")?.parse::<u64>().ok()?;
            let page = format!("/articles/{number}");
            pages
                .iter()
                .any(|cleared| cleared.page == page && cleared.cleared_by.is_none())
                .then(|| Finding {
                    check: "uncleared_page".to_owned(),
                    at: page,
                    expected: None,
                    observed: id.clone().into(),
                })
        })
        .collect();
    TierMoveAnswer {
        answer: answer.refuted_by(findings),
        pages,
        requests,
    }
}
