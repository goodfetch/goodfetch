//! Pages that declare their language in a meta tag, in the `Content-Language`
//! header or on their root element, and contradict it in another place (ADR
//! 0123). Each shows one amount that is a thousand times smaller read in
//! English than in German, so the language a tool takes shows in the value.

use super::FAMILY_PLACE;
use crate::exchange::Reply;
use crate::generator;
use crate::task::Record;

/// The path of the index of the pages.
pub(super) const INDEX_PATH: &str = "/documents";

pub(super) const DESCRIPTION: &str = "Crawl the pages this index links and return one Record \
    per page with the fields `id` (the article's `data-id`), `language` (the page's language: \
    its root element's `lang`, else the one tag of its `content-language` meta tag, else the \
    one tag of its `Content-Language` header, else `und`) and `amount` (the number the page \
    shows, read in that language, as a plain decimal).";

/// The generator places of a page's values, after the family's, this
/// submodule's and the page's own.
const PLACE: u64 = 1;
const LANGUAGE_PLACE: u64 = 1;
const THOUSANDS_PLACE: u64 = 2;
const REST_PLACE: u64 = 3;
const LAST_PLACE: u64 = 4;

/// The two languages pages are in, whose digit groups and decimals swap marks.
const LANGUAGES: [&str; 2] = ["de", "en"];

/// What a place on a page declares.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Declares {
    /// Nothing: it is absent.
    Nothing,
    /// The page's right language.
    Right,
    /// The other language.
    Wrong,
    /// Two tags, the wrong language's first, which name no one language.
    Both,
}

impl Declares {
    fn value(self, right: usize) -> Option<String> {
        let (right, wrong) = (LANGUAGES[right], LANGUAGES[1 - right]);
        match self {
            Self::Nothing => None,
            Self::Right => Some(right.to_owned()),
            Self::Wrong => Some(wrong.to_owned()),
            Self::Both => Some(format!("{wrong}, {right}")),
        }
    }
}

/// A page of the index, by its ID, and what each of its places declares.
struct Page {
    id: &'static str,
    root: Declares,
    meta: Declares,
    header: Declares,
}

static PAGES: [Page; 6] = [
    Page {
        id: "meta",
        root: Declares::Nothing,
        meta: Declares::Right,
        header: Declares::Nothing,
    },
    Page {
        id: "header",
        root: Declares::Nothing,
        meta: Declares::Nothing,
        header: Declares::Right,
    },
    Page {
        id: "meta-over-header",
        root: Declares::Nothing,
        meta: Declares::Right,
        header: Declares::Wrong,
    },
    Page {
        id: "root-over-both",
        root: Declares::Right,
        meta: Declares::Wrong,
        header: Declares::Wrong,
    },
    Page {
        id: "two-tags-in-meta",
        root: Declares::Nothing,
        meta: Declares::Both,
        header: Declares::Right,
    },
    Page {
        id: "two-tags-in-header",
        root: Declares::Nothing,
        meta: Declares::Nothing,
        header: Declares::Both,
    },
];

/// The values a seed gives the page `index` of [`PAGES`].
struct Drawn {
    /// The place of its right language in [`LANGUAGES`].
    language: usize,
    /// The digits before the mark and the three after it, which end in no
    /// zero, so both readings print every digit.
    thousands: u64,
    rest: u64,
}

fn drawn(seed: u64, index: usize) -> Drawn {
    let below =
        |value, bound| generator::below(seed, &[FAMILY_PLACE, PLACE, index as u64, value], bound);
    // The first two pages' languages differ, so every seed has a page in each.
    let language = match index {
        1 => 1 - drawn(seed, 0).language,
        _ => usize::from(below(LANGUAGE_PLACE, 2) == 1),
    };
    Drawn {
        language,
        thousands: 1 + below(THOUSANDS_PLACE, 9),
        rest: 100 + 10 * below(REST_PLACE, 90) + 1 + below(LAST_PLACE, 9),
    }
}

impl Page {
    /// Whether the page declares one language anywhere.
    fn has_language(&self) -> bool {
        [self.root, self.meta, self.header].contains(&Declares::Right)
    }

    fn path(&self) -> String {
        format!("{INDEX_PATH}/{}", self.id)
    }

    /// The amount as the page shows it. The page of no language shows one
    /// that reads alike in any.
    fn shown(&self, drawn: &Drawn) -> String {
        let Drawn {
            thousands, rest, ..
        } = drawn;
        if self.has_language() {
            format!("{thousands}.{rest}")
        } else {
            format!("{thousands}{rest}")
        }
    }

    fn record(&self, drawn: &Drawn) -> Record {
        let (language, amount) = if self.has_language() {
            let Drawn {
                language,
                thousands,
                rest,
            } = drawn;
            let language = LANGUAGES[*language];
            let mark = if language == "de" { "" } else { "." };
            (language, format!("{thousands}{mark}{rest}"))
        } else {
            ("und", self.shown(drawn))
        };
        Record::from([
            ("id", self.id.to_owned()),
            ("language", language.to_owned()),
            ("amount", amount),
        ])
    }

    fn reply(&self, drawn: &Drawn) -> Reply {
        let root = self
            .root
            .value(drawn.language)
            .map(|language| format!(" lang=\"{language}\""))
            .unwrap_or_default();
        let meta = self
            .meta
            .value(drawn.language)
            .map(|tags| format!("<meta http-equiv=\"content-language\" content=\"{tags}\">"))
            .unwrap_or_default();
        let id = self.id;
        let shown = self.shown(drawn);
        let mut reply = Reply::html(format!(
            "<!doctype html>\n<html{root}>\n<head><meta charset=\"utf-8\">{meta}<title>{id}</title></head>\n<body>\n<main>\n<article data-id=\"{id}\">\n<h1>{id}</h1>\n<p><span class=\"amount\">{shown}</span></p>\n</article>\n</main>\n</body>\n</html>\n"
        ));
        if let Some(tags) = self.header.value(drawn.language) {
            reply.headers.push(("content-language", tags));
        }
        reply
    }
}

/// Answers a request for the index or one of its pages.
pub(super) fn respond(seed: u64, path: &str) -> Option<Reply> {
    if path == INDEX_PATH {
        return Some(super::index("Documents", PAGES.iter().map(Page::path)));
    }
    PAGES
        .iter()
        .enumerate()
        .find(|(_, page)| page.path() == path)
        .map(|(index, page)| page.reply(&drawn(seed, index)))
}

/// The expected Record of each page.
pub(super) fn records(seed: u64) -> Vec<Record> {
    PAGES
        .iter()
        .enumerate()
        .map(|(index, page)| page.record(&drawn(seed, index)))
        .collect()
}
