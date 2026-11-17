//! The extraction scenario's output check (#2136): what each tool wrote of
//! each page of the article site, its title and its Main content, held to
//! the right answer the Proving ground serves for that page, so the
//! scenario's figures compare tools that read the same thing.
//!
//! A page reads right when:
//!
//! - its title equals the right one once white space, Unicode's included,
//!   is collapsed on what the tool wrote. Nothing is decoded here: each
//!   tool decodes character references inside its own timed process;
//! - its text, white space collapsed the same way, holds every paragraph
//!   of the article;
//! - its text holds no marker of the page's boilerplate, its header, nav,
//!   aside and footer;
//! - its text does not hold the ad's marker.
//!
//! The last one is counted apart too ([`Extracted`]'s `ad_allowed`): the
//! ad sits inside the article, where only a Clean-up rule removes it, so a
//! tool with no ad removal, and Foxhound's row with its Filter lists off,
//! are read without it.
//!
//! The handled crawl (#2317) holds each page to its answers field by field
//! instead ([`check_fields`]): the twenty fields its handler reads.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The largest article site whose right answers the Proving ground serves
/// (`ANSWERED_PAGES` of its `crawl/comparison/article.rs`).
pub(crate) const ANSWERED_PAGES: u64 = 10_000;

/// The right answers of an article site, as the Proving ground's `GET
/// /article-answers/<size>` serves them: test support's `ArticleAnswers`,
/// which this crate's tests alone may read (a unit test holds the two
/// equal).
#[derive(Debug, Deserialize)]
pub(crate) struct Answers {
    pages: Vec<Answer>,
    /// The link every page's nav holds to the site's first page.
    home: Link,
    /// The markers of each page's header, nav, aside and footer, in this
    /// order.
    boilerplate: Vec<String>,
    ad: String,
}

/// One page's right answer: its path, its title as a reader reads it, its
/// article's paragraphs in order, its article's ID, its heading's name and
/// its nav's links to its children.
#[derive(Debug, Deserialize)]
struct Answer {
    path: String,
    title: String,
    paragraphs: Vec<String>,
    id: String,
    name: String,
    children: Vec<Link>,
}

/// One link of a page's nav: to a child page, or home.
#[derive(Debug, Deserialize)]
struct Link {
    path: String,
    name: String,
}

/// What a tool wrote of one page of the handled crawl (#2317): its URL and
/// its fields by name.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct Fielded {
    pub(crate) url: String,
    #[serde(flatten)]
    pub(crate) fields: Map<String, Value>,
}

/// What a tool wrote of one page.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct Page {
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) text: Option<String>,
}

/// Where a tool leaves its pages in its data folder.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Written {
    /// One JSON object a line, in the file of this name.
    Lines(&'static str),
    /// One JSON file a page in Crawlee's default Dataset,
    /// `storage/datasets/default/`, which crawlee-rs writes the same.
    Dataset,
}

/// Crawlee's default Dataset, in the folder a crawl ran in.
const DATASET: &str = "storage/datasets/default";

/// The pages of the site that differ from their right answer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Differing {
    /// How many.
    pub(crate) pages: u64,
    /// The first of them in the site's order, with what differs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) first: Option<String>,
}

impl Differing {
    fn add(&mut self, difference: String) {
        self.pages += 1;
        self.first.get_or_insert(difference);
    }
}

/// What the output check found of one crawl.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Extracted {
    /// The pages that differ from the whole right answer.
    pub(crate) whole: Differing,
    /// Those that still differ once the ad's marker is allowed in the
    /// text: what a tool that removes no ad is held to.
    pub(crate) ad_allowed: Differing,
}

/// The right answers of the article site of `pages` pages, from the
/// Proving ground at `address`.
pub(crate) fn answers(address: &str, pages: u64) -> Result<Answers> {
    ureq::get(&format!("http://{address}/article-answers/{pages}"))
        .call()
        .context("reading the Proving ground's article answers")?
        .body_mut()
        .with_config()
        .limit(u64::MAX)
        .read_json()
        .context("parsing the Proving ground's article answers")
}

/// The pages a tool left in `home`, the folder it ran in.
pub(crate) fn read<T: DeserializeOwned>(written: Written, home: &Path) -> Result<Vec<T>> {
    match written {
        Written::Lines(name) => {
            let file = home.join(name);
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            text.lines()
                .map(|line| {
                    serde_json::from_str(line)
                        .with_context(|| format!("parsing a line of {}", file.display()))
                })
                .collect()
        }
        Written::Dataset => {
            let folder = home.join(DATASET);
            let mut files: Vec<_> = std::fs::read_dir(&folder)
                .with_context(|| format!("reading {}", folder.display()))?
                .flatten()
                .map(|entry| entry.path())
                // A page's file is named by its number; the Dataset's own
                // files start with `__`.
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "json")
                        && path
                            .file_name()
                            .is_some_and(|name| !name.to_string_lossy().starts_with("__"))
                })
                .collect();
            files.sort();
            files
                .iter()
                .map(|file| {
                    let text = std::fs::read_to_string(file)
                        .with_context(|| format!("reading {}", file.display()))?;
                    serde_json::from_str(&text)
                        .with_context(|| format!("parsing {}", file.display()))
                })
                .collect()
        }
    }
}

/// White space collapsed to single spaces, Unicode's included, and
/// trimmed.
fn collapsed(text: &str) -> String {
    let mut collapsed = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !collapsed.is_empty() {
            collapsed.push(' ');
        }
        collapsed.push_str(word);
    }
    collapsed
}

/// A URL's path: what follows its authority.
fn path_of(url: &str) -> &str {
    url.split_once("://")
        .and_then(|(_, rest)| rest.find('/').map(|slash| &rest[slash..]))
        .unwrap_or(url)
}

/// How a written page differs from its right answer, if it does: first by
/// anything but the ad's marker, then by the ad's marker.
fn differences(answers: &Answers, answer: &Answer, page: Option<&Page>) -> [Option<String>; 2] {
    let Some(page) = page else {
        return [Some("no page written".to_owned()), None];
    };
    let text = page.text.as_deref().map(collapsed);
    let other = match (&page.title, &text) {
        (None, _) => Some(format!("no title, not {:?}", answer.title)),
        (Some(title), _) if collapsed(title) != answer.title => {
            Some(format!("title {title:?}, not {:?}", answer.title))
        }
        (_, None) => Some("no text".to_owned()),
        (_, Some(text)) => answer
            .paragraphs
            .iter()
            .position(|paragraph| !text.contains(paragraph.as_str()))
            .map(|index| format!("the text lacks paragraph {}", index + 1))
            .or_else(|| {
                answers
                    .boilerplate
                    .iter()
                    .find(|marker| text.contains(marker.as_str()))
                    .map(|marker| format!("the text holds {marker:?}"))
            }),
    };
    let ad = text
        .is_some_and(|text| text.contains(&answers.ad))
        .then(|| format!("the text holds the ad's {:?}", answers.ad));
    [other, ad]
}

/// Holds what a tool wrote, `pages`, to the site's right `answers`.
pub(crate) fn check(answers: &Answers, pages: &[Page]) -> Extracted {
    // A page written twice is read as first written.
    let mut written: HashMap<&str, &Page> = HashMap::with_capacity(pages.len());
    for page in pages {
        written.entry(path_of(&page.url)).or_insert(page);
    }
    let mut extracted = Extracted::default();
    for answer in &answers.pages {
        let [other, ad] = differences(answers, answer, written.get(answer.path.as_str()).copied());
        let named = |difference: &String| format!("{}: {difference}", answer.path);
        if let Some(difference) = other.as_ref().or(ad.as_ref()) {
            extracted.whole.add(named(difference));
        }
        if let Some(difference) = &other {
            extracted.ad_allowed.add(named(difference));
        }
    }
    extracted
}

/// How a field of the handled crawl is compared: as text, white space
/// collapsed, or as a link, by its path.
#[derive(Clone, Copy)]
enum Kind {
    Text,
    Href,
}

/// The twenty fields of a page of the article site and each one's answer,
/// `None` where the page has no element for it: its title, its heading's
/// name and the article's `data-id`, its nav's home link and the label,
/// the header's, nav's and footer's markers, its first two children's
/// links and names, and its article's first eight paragraphs.
fn fields_of(answers: &Answers, answer: &Answer) -> Vec<(String, Kind, Option<String>)> {
    let marker = |index: usize| answers.boilerplate.get(index).cloned();
    let child = |index: usize| answer.children.get(index);
    let mut fields = vec![
        ("title".to_owned(), Kind::Text, Some(answer.title.clone())),
        ("name".to_owned(), Kind::Text, Some(answer.name.clone())),
        ("id".to_owned(), Kind::Text, Some(answer.id.clone())),
        (
            "home".to_owned(),
            Kind::Href,
            Some(answers.home.path.clone()),
        ),
        (
            "home_label".to_owned(),
            Kind::Text,
            Some(answers.home.name.clone()),
        ),
        ("header".to_owned(), Kind::Text, marker(0)),
        ("nav".to_owned(), Kind::Text, marker(1)),
        ("footer".to_owned(), Kind::Text, marker(3)),
    ];
    for (index, ordinal) in [(0, "first"), (1, "second")] {
        let link = child(index);
        fields.push((
            format!("{ordinal}_child"),
            Kind::Href,
            link.map(|link| link.path.clone()),
        ));
        fields.push((
            format!("{ordinal}_child_name"),
            Kind::Text,
            link.map(|link| link.name.clone()),
        ));
    }
    for k in 1..=8 {
        fields.push((
            format!("paragraph_{k}"),
            Kind::Text,
            answer.paragraphs.get(k - 1).cloned(),
        ));
    }
    fields
}

/// How a page's written fields differ from its answers, if they do: the
/// first field that differs.
fn field_difference(answers: &Answers, answer: &Answer, page: Option<&Fielded>) -> Option<String> {
    let Some(page) = page else {
        return Some("no page written".to_owned());
    };
    fields_of(answers, answer)
        .into_iter()
        .find_map(|(name, kind, expected)| {
            let written = page.fields.get(&name).filter(|value| !value.is_null());
            let read = written.map(|value| match (value.as_str(), kind) {
                (Some(text), Kind::Text) => Some(collapsed(text)),
                (Some(href), Kind::Href) => Some(path_of(href).to_owned()),
                (None, _) => None,
            });
            let right = match (&read, &expected) {
                (None, None) => true,
                (Some(Some(read)), Some(expected)) => read == expected,
                _ => false,
            };
            (!right).then(|| {
                let got = written.map_or_else(|| "none".to_owned(), ToString::to_string);
                let want = expected.map_or_else(|| "none".to_owned(), |text| format!("{text:?}"));
                format!("{name} {got}, not {want}")
            })
        })
}

/// Holds what a tool wrote of the handled crawl, `pages`, to the site's
/// `answers`, field by field (#2317). No ad is among the fields: both
/// counts are the same.
pub(crate) fn check_fields(answers: &Answers, pages: &[Fielded]) -> Extracted {
    // A page written twice is read as first written.
    let mut written: HashMap<&str, &Fielded> = HashMap::with_capacity(pages.len());
    for page in pages {
        written.entry(path_of(&page.url)).or_insert(page);
    }
    let mut differing = Differing::default();
    for answer in &answers.pages {
        let page = written.get(answer.path.as_str()).copied();
        if let Some(difference) = field_difference(answers, answer, page) {
            differing.add(format!("{}: {difference}", answer.path));
        }
    }
    Extracted {
        whole: differing.clone(),
        ad_allowed: differing,
    }
}

#[cfg(test)]
mod tests;
