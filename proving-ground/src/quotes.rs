//! The `quotes` family on `quotes.test` (#1535): the dependent form of golden
//! scenario 01, the Proving ground's own in place of
//! `quotes.toscrape.com/search.aspx`. `/search` offers the authors, and its
//! tag list stays empty until an author is chosen: the author list's
//! `onchange` posts the form back to `/search`, which reloads it with that
//! author's tags and a new hidden `state`. Search posts the form to
//! `/filter`, which refuses a tag the chosen author is not offered and a
//! `state` minted for another author, and rejects the typed confirmation the
//! first time with a message saying what to type; the form it reloads takes
//! that answer, and only then shows the quotes. Authors and tags are fixed;
//! each author's tags, the quotes and the confirmation come from the seed,
//! and the family keeps no state: `state` carries the form's step.

use std::fmt::Write as _;

use hyper::{Method, StatusCode};

use crate::exchange::{Reply, Request};
use crate::generator;
use crate::task::{Entry, Expected, Kind, Record, Task, origin};

/// The host name the family is served on.
pub(crate) const HOST: &str = "quotes.test";

const FAMILY: &str = "quotes";

const AUTHORS: [&str; 6] = [
    "Ada Quill",
    "Bram Holt",
    "Cleo Marsh",
    "Dorian Vale",
    "Esme Rook",
    "Felix Thorne",
];
/// In the order the tag list offers them.
const TAGS: [&str; 8] = [
    "courage",
    "friendship",
    "hope",
    "humor",
    "inspirational",
    "learning",
    "life",
    "truth",
];
/// How many tags each author is offered.
const TAGS_PER_AUTHOR: usize = 4;
/// The author and tag `quotes/1` names, by index.
const TASK_AUTHOR: usize = 2;
const TASK_TAG: usize = 4;
/// The most quotes an author has under one tag; each has at least one.
const MOST_QUOTES: u64 = 3;

const OPENERS: [&str; 8] = [
    "Every",
    "No",
    "One",
    "A quiet",
    "The first",
    "Each small",
    "An honest",
    "The last",
];
const SUBJECTS: [&str; 8] = [
    "step", "doubt", "promise", "morning", "question", "mistake", "friend", "song",
];
const VERBS: [&str; 8] = [
    "carries", "outlasts", "teaches", "opens", "mends", "lights", "answers", "outruns",
];
const OBJECTS: [&str; 8] = [
    "the longest road",
    "a closed door",
    "the heaviest day",
    "an old fear",
    "the patient heart",
    "a borrowed hour",
    "the open sea",
    "a careful mind",
];

/// The generator places of the family's values: the family's own first, then
/// the author's number and the tag's (each from 1, 0 for none) and the
/// value's, with the quote's number and the word's last where a value has
/// several.
const FAMILY_PLACE: u64 = 42;
const TAGS_PLACE: u64 = 1;
const STATE_PLACE: u64 = 2;
const CODE_PLACE: u64 = 3;
const COUNT_PLACE: u64 = 4;
const QUOTE_PLACE: u64 = 5;
const ID_PLACE: u64 = 6;

fn place(author: usize, tag: Option<usize>, value: u64) -> [u64; 4] {
    let number = |index: usize| u64::try_from(index).expect("invariant: a short list's index") + 1;
    [FAMILY_PLACE, number(author), tag.map_or(0, number), value]
}

fn index(value: u64) -> usize {
    usize::try_from(value).expect("invariant: a drawn index is below a short list's length")
}

/// The indices of the tags `author` is offered, in [`TAGS`]'s order. The
/// task's author is always offered the task's tag.
fn tags(seed: u64, author: usize) -> Vec<usize> {
    let mut tags = generator::distinct(
        seed,
        &place(author, None, TAGS_PLACE),
        TAGS.len(),
        TAGS_PER_AUTHOR,
    );
    if author == TASK_AUTHOR && !tags.contains(&TASK_TAG) {
        tags[TAGS_PER_AUTHOR - 1] = TASK_TAG;
    }
    tags.sort_unstable();
    tags
}

/// The form's `state` once `author` is chosen, or once the confirmation of
/// `tag` was asked.
fn state(seed: u64, author: usize, tag: Option<usize>) -> String {
    format!(
        "{:016x}",
        generator::draw(seed, &place(author, tag, STATE_PLACE))
    )
}

/// What the confirmation of `tag` for `author` must be.
fn confirmation(seed: u64, author: usize, tag: usize) -> String {
    let digits = generator::below(seed, &place(author, Some(tag), CODE_PLACE), 10_000);
    format!("{}-{digits:04}", TAGS[tag])
}

/// One quote of the results.
struct Quote {
    id: String,
    text: String,
}

/// The quotes of `author` under `tag`.
fn quotes(seed: u64, author: usize, tag: usize) -> Vec<Quote> {
    let count = 1 + generator::below(seed, &place(author, Some(tag), COUNT_PLACE), MOST_QUOTES);
    (0..count)
        .map(|number| {
            let mut id_at = place(author, Some(tag), ID_PLACE).to_vec();
            id_at.push(number);
            let word = |part: u64, bound: usize| {
                let mut at = place(author, Some(tag), QUOTE_PLACE).to_vec();
                at.extend([number, part]);
                index(generator::below(
                    seed,
                    &at,
                    u64::try_from(bound).expect("invariant: a short list's length"),
                ))
            };
            Quote {
                id: format!("{:016x}", generator::draw(seed, &id_at)),
                text: format!(
                    "{} {} {} {}.",
                    OPENERS[word(0, OPENERS.len())],
                    SUBJECTS[word(1, SUBJECTS.len())],
                    VERBS[word(2, VERBS.len())],
                    OBJECTS[word(3, OBJECTS.len())]
                ),
            }
        })
        .collect()
}

/// Answers a request on the family's host names.
pub(crate) fn respond(seed: u64, request: &Request<'_>) -> Reply {
    match (request.method, request.path) {
        (&Method::GET, "/search") => Reply::html(form_page(seed, &Form::default())),
        (&Method::POST, "/search") => choose_author(seed, request),
        (&Method::POST, "/filter") => filter(seed, request),
        _ => Reply::not_found(),
    }
}

/// The value of the field `name` in a submitted form.
fn field(request: &Request<'_>, name: &str) -> Option<String> {
    form_urlencoded::parse(request.body)
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

fn author_field(request: &Request<'_>) -> Option<usize> {
    let author = field(request, "author")?;
    AUTHORS.iter().position(|name| *name == author)
}

/// The form as one page shows it.
#[derive(Default)]
struct Form<'a> {
    author: Option<usize>,
    tag: Option<usize>,
    error: Option<&'a str>,
}

fn choose_author(seed: u64, request: &Request<'_>) -> Reply {
    match author_field(request) {
        Some(author) => Reply::html(form_page(
            seed,
            &Form {
                author: Some(author),
                ..Form::default()
            },
        )),
        None => refused(seed, &Form::default(), "Choose an author from the list."),
    }
}

fn refused(seed: u64, form: &Form<'_>, error: &str) -> Reply {
    Reply::html_with_status(
        StatusCode::BAD_REQUEST,
        form_page(
            seed,
            &Form {
                error: Some(error),
                ..*form
            },
        ),
    )
}

fn filter(seed: u64, request: &Request<'_>) -> Reply {
    let Some(author) = author_field(request) else {
        return refused(seed, &Form::default(), "Choose an author from the list.");
    };
    let chosen = Form {
        author: Some(author),
        ..Form::default()
    };
    let tag = field(request, "tag").and_then(|tag| TAGS.iter().position(|name| *name == tag));
    let sent = field(request, "state").unwrap_or_default();
    let asked = tag.is_some_and(|tag| sent == state(seed, author, Some(tag)));
    if !asked && sent != state(seed, author, None) {
        return refused(
            seed,
            &chosen,
            "The author changed since its tags were loaded: choose the author again.",
        );
    }
    let Some(tag) = tag else {
        return refused(seed, &chosen, "Choose a tag from the list.");
    };
    if !tags(seed, author).contains(&tag) {
        let error = format!(
            "The tag {} is not offered for {}.",
            TAGS[tag], AUTHORS[author]
        );
        return refused(seed, &chosen, &error);
    }
    let expected = confirmation(seed, author, tag);
    if asked && field(request, "confirm").as_deref() == Some(expected.as_str()) {
        return Reply::html(results_page(seed, author, tag));
    }
    let error = format!("Type {expected} to confirm the tag.");
    Reply::html_with_status(
        StatusCode::UNPROCESSABLE_ENTITY,
        form_page(
            seed,
            &Form {
                author: Some(author),
                tag: Some(tag),
                error: Some(&error),
            },
        ),
    )
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{title}</title></head>\n<body>\n<main>\n{body}</main>\n</body>\n</html>\n"
    )
}

fn option(value: &str, selected: bool) -> String {
    let selected = if selected { " selected" } else { "" };
    format!("<option value=\"{value}\"{selected}>{value}</option>\n")
}

/// The search form: the authors, the chosen author's tags, and the `state`
/// of its step, with the error of the last submission, if any.
fn form_page(seed: u64, form: &Form<'_>) -> String {
    let mut authors = String::from("<option value=\"\">----------</option>\n");
    for (index, author) in AUTHORS.iter().enumerate() {
        authors.push_str(&option(author, form.author == Some(index)));
    }
    let mut tag_list = String::from("<option value=\"\">----------</option>\n");
    let (disabled, state) = match form.author {
        None => (
            " disabled",
            format!(
                "{:016x}",
                generator::draw(seed, &[FAMILY_PLACE, 0, 0, STATE_PLACE])
            ),
        ),
        Some(author) => {
            for tag in tags(seed, author) {
                tag_list.push_str(&option(TAGS[tag], form.tag == Some(tag)));
            }
            ("", state(seed, author, form.tag))
        }
    };
    let error = form.error.map_or_else(String::new, |error| {
        format!("<p class=\"error\" role=\"alert\">{error}</p>\n")
    });
    page(
        "Search quotes",
        &format!(
            "<h1>Search quotes</h1>\n{error}<form method=\"post\" action=\"/search\">\n<input type=\"hidden\" name=\"state\" value=\"{state}\">\n<label>Author <select name=\"author\" onchange=\"this.form.submit()\">\n{authors}</select></label>\n<label>Tag <select name=\"tag\"{disabled}>\n{tag_list}</select></label>\n<label>Confirm the tag <input name=\"confirm\" autocomplete=\"off\"></label>\n<button type=\"submit\" formaction=\"/filter\">Search</button>\n</form>\n"
        ),
    )
}

fn results_page(seed: u64, author: usize, tag: usize) -> String {
    let (author_name, tag_name) = (AUTHORS[author], TAGS[tag]);
    let list = quotes(seed, author, tag)
        .iter()
        .fold(String::new(), |mut list, Quote { id, text }| {
            writeln!(
                list,
                "<li class=\"quote\" data-id=\"{id}\"><span class=\"text\">{text}</span> <span class=\"author\">{author_name}</span> <span class=\"tag\">{tag_name}</span></li>"
            )
            .expect("invariant: writing to a String succeeds");
            list
        });
    page(
        "Quotes",
        &format!(
            "<h1>Quotes by {author_name} tagged {tag_name}</h1>\n<ul class=\"quotes\">\n{list}</ul>\n<a href=\"/search\">New search</a>\n"
        ),
    )
}

/// The host name of the family's task `ordinal`: `1.quotes.test` for
/// `quotes/1`.
fn task_host(ordinal: u32) -> String {
    format!("{ordinal}.{HOST}")
}

/// The family's tasks for `seed`, their start URLs on their own host names
/// and `port`. Ordinals are append-only (ADR 0063).
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let records = quotes(seed, TASK_AUTHOR, TASK_TAG)
        .into_iter()
        .map(|Quote { id, text }| {
            Record::from([
                ("id", id),
                ("text", text),
                ("author", AUTHORS[TASK_AUTHOR].to_owned()),
                ("tag", TAGS[TASK_TAG].to_owned()),
            ])
        })
        .collect();
    vec![Task {
        host: task_host(1),
        entry: Entry {
            id: format!("{FAMILY}/1"),
            family: FAMILY,
            kind: Kind::Crawl,
            start_url: format!("{}/search", origin(&task_host(1), port)),
            description: "Choose the author \"Cleo Marsh\": the page reloads and the tag list \
                then offers that author's tags. Choose the tag \"inspirational\", type the tag \
                to confirm it, press Search, and return one Record per quote shown, with the \
                fields `id` (its `data-id`), `text`, `author` and `tag`. The page may reject the \
                confirmation and say what to type: type that, and press Search again.",
            account: None,
            time_steps: None,
        },
        expected: Expected::records(records),
    }]
}
