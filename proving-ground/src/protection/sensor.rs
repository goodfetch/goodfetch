//! The sensor page: an article behind a check as Akamai Bot Manager's are,
//! which takes only sensor data made for its own page, cookies, script and
//! client. The solver-service stand-in (`solvers`) makes the sensor data, as
//! the real service makes real ones; the page's own script sends none.

use hyper::body::Bytes;
use hyper::header::USER_AGENT;
use hyper::{Method, StatusCode};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest as _, Sha256};

use super::{FAMILY_PLACE, SENSOR_PLACE};
use crate::exchange::{Reply, Request};
use crate::generator;

/// The page's path.
pub(super) const PATH: &str = "/sensor";

/// The path its script is served at and posts the sensor data to.
const SCRIPT_PATH: &str = "/sensor/script.js";

/// The cookie that says whether the client's sensor data was accepted, and
/// the one beside it.
const ABCK: &str = "_abck";
const BM_SZ: &str = "bm_sz";

/// The values drawn at the sensor's generator place.
const SCRIPT_VALUE: u64 = 0;
const ABCK_VALUE: u64 = 1;
const BM_SZ_VALUE: u64 = 2;
const ACCEPTED_VALUE: u64 = 3;

/// What sensor data is made for.
pub(crate) struct Sensed<'a> {
    pub(crate) abck: &'a str,
    pub(crate) bm_sz: &'a str,
    /// The page's host name, without its port, and its path.
    pub(crate) host: &'a str,
    pub(crate) path: &'a str,
    pub(crate) user_agent: &'a str,
    /// The SHA-256 of the page's script, in lowercase hex.
    pub(crate) script_hash: &'a str,
}

/// The body a script posts its sensor data in.
#[derive(Deserialize)]
struct Sent {
    sensor_data: String,
}

/// 32 hexadecimal digits of the value `value` for `seed`.
fn drawn(seed: u64, value: u64) -> String {
    let half = |half| generator::draw(seed, &[FAMILY_PLACE, 0, SENSOR_PLACE, value, half]);
    format!("{:016x}{:016x}", half(0), half(1))
}

/// The page's script for `seed`.
fn script(seed: u64) -> String {
    format!(
        "window.bmak = {{ version: 3, key: \"{}\" }};\n",
        drawn(seed, SCRIPT_VALUE)
    )
}

/// The SHA-256 of the page's script for `seed`, in lowercase hex.
pub(crate) fn script_hash(seed: u64) -> String {
    hash(&script(seed))
}

/// The SHA-256 of `script`, in lowercase hex.
pub(crate) fn hash(script: &str) -> String {
    data_encoding::HEXLOWER.encode(&Sha256::digest(script))
}

/// The `_abck` cookie of a client whose sensor data was not accepted yet.
fn pending(seed: u64) -> String {
    let drawn = drawn(seed, ABCK_VALUE).to_uppercase();
    format!("{drawn}~-1~YAAQ{drawn}~-1~-1~-1")
}

/// The `_abck` cookie of a client whose sensor data was accepted.
fn accepted(seed: u64) -> String {
    let drawn = drawn(seed, ABCK_VALUE).to_uppercase();
    format!(
        "{drawn}~0~YAAQ{}~-1~-1~-1",
        self::drawn(seed, ACCEPTED_VALUE)
    )
}

/// The `bm_sz` cookie for `seed`.
fn bm_sz(seed: u64) -> String {
    format!("{}~YAAQ", drawn(seed, BM_SZ_VALUE).to_uppercase())
}

/// The sensor data `number` for `sensed`: the number, then a digest only the
/// Proving ground's seed gives.
pub(crate) fn data(seed: u64, sensed: &Sensed<'_>, number: u64) -> String {
    let mut digest = Sha256::new();
    for part in [
        seed.to_string().as_str(),
        sensed.abck,
        sensed.bm_sz,
        sensed.host,
        sensed.path,
        sensed.user_agent,
        sensed.script_hash,
        &number.to_string(),
    ] {
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    let mac = data_encoding::HEXLOWER.encode(&digest.finalize());
    format!("3;0;1;0;{number};{mac}")
}

/// Whether the page on `host` accepts the sensor data `request` posts: data
/// made for its own page and script and for the request's cookies and user
/// agent.
fn accepts(seed: u64, host: &str, request: &Request<'_>) -> bool {
    let Ok(Sent { sensor_data }) = serde_json::from_slice(request.body) else {
        return false;
    };
    let user_agent = request
        .headers
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok());
    let (Some(abck), Some(bm_sz), Some(user_agent)) =
        (request.cookie(ABCK), request.cookie(BM_SZ), user_agent)
    else {
        return false;
    };
    let number = sensor_data
        .strip_prefix("3;0;1;0;")
        .and_then(|rest| rest.split_once(';'))
        .and_then(|(number, _)| number.parse().ok());
    let sensed = Sensed {
        abck,
        bm_sz,
        host,
        path: PATH,
        user_agent,
        script_hash: &script_hash(seed),
    };
    abck == pending(seed)
        && bm_sz == self::bm_sz(seed)
        && number.is_some_and(|number| sensor_data == data(seed, &sensed, number))
}

/// Answers a request for the page on `host`, whose article is `article`, or
/// for its script; `None` for any other.
pub(super) fn respond(
    seed: u64,
    host: &str,
    request: &Request<'_>,
    article: impl FnOnce() -> String,
) -> Option<Reply> {
    Some(match (request.method, request.path) {
        (&Method::GET, PATH) if request.cookie(ABCK) == Some(accepted(seed).as_str()) => {
            Reply::html(article())
        }
        (&Method::GET, PATH) => Reply::html_with_status(StatusCode::FORBIDDEN, page())
            .set_cookie(ABCK, &pending(seed))
            .set_cookie(BM_SZ, &bm_sz(seed)),
        (&Method::GET, SCRIPT_PATH) => Reply {
            status: StatusCode::OK,
            content_type: "text/javascript; charset=utf-8",
            headers: Vec::new(),
            body: Bytes::from(script(seed)),
        },
        (&Method::POST, SCRIPT_PATH) => {
            let success = accepts(seed, host, request);
            let abck = if success {
                accepted(seed)
            } else {
                pending(seed)
            };
            Reply::json(StatusCode::CREATED, &json!({ "success": success })).set_cookie(ABCK, &abck)
        }
        _ => return None,
    })
}

/// The page of a client whose sensor data was not accepted yet.
fn page() -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Access Denied</title></head>\n<body>\n<main id=\"sensor\">\n<h1>Access Denied</h1>\n<p>You don't have permission to access this page.</p>\n</main>\n<script src=\"{SCRIPT_PATH}\"></script>\n</body>\n</html>\n"
    )
}
