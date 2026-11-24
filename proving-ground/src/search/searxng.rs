//! The `/search` of a SearXNG instance whose settings allow the `html` and
//! `json` formats: the parameters it takes, in a query or a posted form, the
//! refusals it gives and the shape of its JSON answer, as SearXNG's
//! `searx/webapp.py`, `searx/webadapter.py` and `searx/webutils.py` write
//! them.

use hyper::{Method, StatusCode};
use serde_json::{Value, json};

use super::{Article, index, results_page};
use crate::exchange::{Reply, Request, form_value};

/// The formats SearXNG knows that this instance's settings leave out.
const REFUSED_FORMATS: [&str; 2] = ["csv", "rss"];

/// What a search answers in.
#[derive(Clone, Copy)]
enum Format {
    Html,
    Json,
}

impl Format {
    /// A refusal of status `400` that says `message`.
    fn refusal(self, message: &str) -> Reply {
        match self {
            Self::Html => Reply::html_with_status(StatusCode::BAD_REQUEST, index(Some(message))),
            Self::Json => Reply::json(StatusCode::BAD_REQUEST, &json!({ "error": message })),
        }
    }

    /// The answer to `query`, which is `results`.
    fn answer(self, query: &str, results: &[Article]) -> Reply {
        match self {
            Self::Html => Reply::html(results_page(query, results)),
            Self::Json => Reply::json(StatusCode::OK, &answer(query, results)),
        }
    }
}

/// Answers a search, whose results `results` gives for a query.
pub(super) fn respond(request: &Request<'_>, results: impl FnOnce(&str) -> Vec<Article>) -> Reply {
    let body = match *request.method {
        Method::POST => std::str::from_utf8(request.body).unwrap_or_default(),
        _ => "",
    };
    let parameter = |name| form_value(body, name).or_else(|| request.parameter(name));

    let format = match parameter("format").as_deref() {
        Some("json") => Format::Json,
        Some(format) if REFUSED_FORMATS.contains(&format) => {
            return Reply::html_with_status(StatusCode::FORBIDDEN, index(Some("Forbidden")));
        }
        _ => Format::Html,
    };
    let Some(query) = parameter("q").filter(|query| !query.is_empty()) else {
        return match format {
            Format::Html => Reply::html(index(None)),
            Format::Json => format.refusal("No query"),
        };
    };
    if let Some(message) = invalid(&parameter) {
        return format.refusal(&message);
    }
    // Every result is on the first page.
    let first_page = parameter("pageno").is_none_or(|page| page.parse() == Ok(1_u64));
    let found = if first_page {
        results(&query)
    } else {
        Vec::new()
    };
    format.answer(&query, &found)
}

/// A parameter's name and whether SearXNG takes a value of it.
type Check<'a> = (&'static str, &'a dyn Fn(&str) -> bool);

/// What SearXNG says of the first parameter whose value it does not take.
fn invalid(parameter: &impl Fn(&'static str) -> Option<String>) -> Option<String> {
    let page = |value: &str| value.parse::<u64>().is_ok_and(|page| page >= 1);
    let language = |value: &str| value == "auto" || is_language(value);
    let safe_search = |value: &str| matches!(value, "0" | "1" | "2");
    let time_range = |value: &str| matches!(value, "" | "None" | "day" | "week" | "month" | "year");
    let checks: [Check<'_>; 4] = [
        ("pageno", &page),
        ("language", &language),
        ("safesearch", &safe_search),
        ("time_range", &time_range),
    ];
    checks.into_iter().find_map(|(name, takes)| {
        let value = parameter(name)?;
        (!takes(&value)).then(|| {
            if value.is_empty() {
                format!("Empty {name} parameter")
            } else {
                format!("Invalid value {value} for parameter {name}")
            }
        })
    })
}

/// Whether SearXNG takes `value` as a language: two or three lower-case
/// letters, then a region of two letters if any.
fn is_language(value: &str) -> bool {
    let (language, region) = value
        .split_once('-')
        .map_or((value, None), |(language, region)| (language, Some(region)));
    (2..=3).contains(&language.len())
        && language.bytes().all(|byte| byte.is_ascii_lowercase())
        && region.is_none_or(|region| {
            region.len() == 2 && region.bytes().all(|byte| byte.is_ascii_alphabetic())
        })
}

/// SearXNG's JSON answer to `query`.
fn answer(query: &str, results: &[Article]) -> Value {
    let results: Vec<Value> = results
        .iter()
        .zip(1_u32..)
        .map(|(article, position)| result(position, article))
        .collect();
    json!({
        "query": query,
        "results": results,
        "answers": [],
        "corrections": [],
        "infoboxes": [],
        "suggestions": [],
        "unresponsive_engines": [],
    })
}

/// The result at `position` of an answer, counted from 1.
fn result(position: u32, article: &Article) -> Value {
    json!({
        "url": article.url,
        "title": article.title,
        "content": article.snippet,
        "publishedDate": null,
        "thumbnail": "",
        "img_src": "",
        "engine": "articles",
        "engines": ["articles"],
        "parsed_url": ["https", "articles.example", format!("/{}", article.id), "", "", ""],
        "template": "default.html",
        "positions": [position],
        "priority": "",
        "score": 1.0 / f64::from(position),
        "category": "general",
    })
}
