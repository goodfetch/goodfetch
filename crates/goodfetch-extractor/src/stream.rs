//! The streaming path of the Page analysis (ADR 0138): when no reader of a
//! page needs its tree, one lol-html pass reads the elements its readers'
//! selectors match, each with its text and the elements holding it, and
//! builds no tree. Its selectors are parsed once per Run ([`Streaming`],
//! [`StreamSelector`]) and lent to each pass.
//!
//! The pass answers only what the tree would: it keeps its own stack of
//! open elements, beside lol-html's, and answers nothing (`None`) for a
//! document whose tree HTML's tree construction would build otherwise
//! than that stack: no `<!doctype html>` (quirks mode), a `<html>`,
//! `<head>` or `<body>` it would imply, an end tag that closes no element
//! or more than one, an element a start tag would close or move (a `<p>`
//! or `<li>` left open, a table's content outside its cells, a second
//! `<a>` inside one), the elements whose parse differs in lol-html
//! (`<noscript>`, which the tree parses as markup, `<template>`,
//! `<select>`, `<plaintext>`), HTML inside SVG or `MathML`, a NUL, or a
//! match inside SVG or `MathML`. Its caller then reads the page's tree.
//! Text and attribute values are decoded as the tokenizer decodes them
//! (the `character_references` module); a selector that tests an
//! attribute's value is not matched on a value lol-html reads undecoded.

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

use goodfetch_counted_work::{CountedWork, Row};
use lol_html::html_content::{Element, TextChunk, TextType};
use lol_html::{
    AsciiCompatibleEncoding, DocumentContentHandlers, ElementContentHandlers, HandlerResult,
    HtmlRewriter, Selector, Settings,
};

use crate::character_references::{Place, decode};

/// A selector the streaming pass matches, parsed once.
#[derive(Debug)]
pub struct StreamSelector {
    selector: Selector,
    /// The attributes whose value it reads, lower-cased: `class` for a
    /// class, `id` for an id, and each attribute an attribute selector
    /// compares, as its text names them (more, never fewer).
    tested: Vec<String>,
}

impl StreamSelector {
    /// `selector` parsed on `work`, none when lol-html reads no such
    /// selector.
    #[must_use]
    pub fn of(selector: &str, work: &CountedWork) -> Option<Self> {
        let parsed = work.compile_stream_selector(Row::PageSelectorCompilations, selector)?;
        Some(Self {
            selector: parsed,
            tested: tested(selector),
        })
    }
}

/// What a pass is given once per Run: the selector of every element, which
/// keeps the pass's stack.
#[derive(Debug)]
pub struct Streaming {
    every: Selector,
}

impl Streaming {
    /// The pass's own selector, parsed on `work`.
    ///
    /// # Panics
    ///
    /// Never: `*` is a selector lol-html reads, as a test shows.
    #[must_use]
    pub fn new(work: &CountedWork) -> Self {
        Self {
            every: work
                .compile_stream_selector(Row::PageSelectorCompilations, "*")
                .expect("invariant: `*` is a selector lol_html reads"),
        }
    }

    /// The elements of `html` each of `queries` matches, in document
    /// order, the first only for a query that asks for it; `None` when the
    /// tree of `html` could hold other elements (the module's list), its
    /// caller then reading the tree.
    #[must_use]
    pub fn read(&self, html: &str, queries: &[Query<'_>]) -> Option<Vec<Vec<Streamed>>> {
        let rest = html.trim_start_matches(|character| WHITESPACE.contains(&character));
        let declared = rest
            .get(..DOCTYPE.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(DOCTYPE));
        if !declared || html.contains('\0') {
            return None;
        }
        let tested = queries
            .iter()
            .flat_map(|query| query.selector.tested.iter().cloned())
            .collect();
        let state = Rc::new(RefCell::new(State::new(queries, tested)));
        let mut settings = Settings::new()
            .with_strict(false)
            // The document is decoded already: a `<meta charset>` changes nothing.
            .with_adjust_charset_on_meta_tag(false)
            .with_encoding(AsciiCompatibleEncoding::utf_8());
        let every = {
            let state = Rc::clone(&state);
            move |element: &mut Element<'_, '_>| started(&state, element)
        };
        settings = settings.append_element_content_handler((
            Cow::Borrowed(&self.every),
            ElementContentHandlers::default().element(every),
        ));
        for (index, query) in queries.iter().enumerate() {
            let state = Rc::clone(&state);
            settings = settings.append_element_content_handler((
                Cow::Borrowed(&query.selector.selector),
                ElementContentHandlers::default().element(
                    move |_: &mut Element<'_, '_>| -> HandlerResult {
                        state.borrow_mut().matched(index)
                    },
                ),
            ));
        }
        let text = {
            let state = Rc::clone(&state);
            move |chunk: &mut TextChunk<'_>| -> HandlerResult {
                let mut state = state.borrow_mut();
                state.flush_if(chunk.text_type())?;
                state.text.push_str(chunk.as_str());
                state.text_type = Some(chunk.text_type());
                Ok(())
            }
        };
        let comments = {
            let state = Rc::clone(&state);
            move |comment: &mut lol_html::html_content::Comment<'_>| -> HandlerResult {
                let mut state = state.borrow_mut();
                state.flush()?;
                state.accounted += end_tags_in(&comment.text());
                state.drop_lf = false;
                Ok(())
            }
        };
        let doctypes = {
            let state = Rc::clone(&state);
            move |_: &mut lol_html::html_content::Doctype<'_>| -> HandlerResult {
                let mut state = state.borrow_mut();
                state.doctypes += 1;
                if state.doctypes > 1 {
                    return Err(Box::new(Refused));
                }
                Ok(())
            }
        };
        settings = settings.append_document_content_handler(
            DocumentContentHandlers::default()
                .text(text)
                .comments(comments)
                .doctype(doctypes),
        );
        let mut rewriter = HtmlRewriter::new(settings, |_: &[u8]| {});
        let written = rewriter
            .write(html.as_bytes())
            .and_then(|()| rewriter.end());
        let state = Rc::try_unwrap(state)
            .ok()
            .expect("invariant: the handlers holding the state ended with the rewriter")
            .into_inner();
        written.ok()?;
        state.finished(end_tags_in(html))
    }
}

/// One selector a pass matches: every element it matches, or the first.
#[derive(Clone, Copy, Debug)]
pub struct Query<'a> {
    pub selector: &'a StreamSelector,
    pub first: bool,
}

/// One element of the document as the tree holds it: its tag name,
/// lower-cased, and its attributes, decoded, in document order, the first
/// of a name kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Held {
    pub name: String,
    pub attributes: Vec<(String, String)>,
}

impl Held {
    /// The value of its attribute `name`, as the tree's `attr` reads it.
    #[must_use]
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(attribute, _)| attribute == name)
            .map(|(_, value)| value.as_str())
    }
}

/// An element a query matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Streamed {
    /// The element, then each element holding it outward, to the root.
    pub held: Vec<Held>,
    /// Its text content, as the tree's `text` reads it.
    pub text: String,
}

/// The error a handler stops the pass with once it knows the tree differs.
#[derive(Debug)]
struct Refused;

impl std::fmt::Display for Refused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the tree differs from the stream's stack")
    }
}

impl std::error::Error for Refused {}

/// `Err` stopping the pass.
fn refused<T>() -> Result<T, Box<dyn std::error::Error + Send + Sync>> {
    Err(Box::new(Refused))
}

/// What the pass reads with: no-quirks mode's doctype.
const DOCTYPE: &str = "<!doctype html>";

/// HTML's ASCII whitespace.
const WHITESPACE: [char; 5] = ['\t', '\n', '\x0C', '\r', ' '];

/// Where the document's tree stands, as tree construction's insertion
/// modes the pass follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    BeforeHtml,
    BeforeHead,
    InHead,
    AfterHead,
    InBody,
    AfterBody,
    AfterHtml,
}

/// An open element.
struct Open {
    held: Held,
    /// Whether it is an HTML element, not an SVG or `MathML` one.
    html: bool,
}

/// The pass's state, which every handler shares.
struct State {
    /// For each query, whether it reads the first match only.
    first: Vec<bool>,
    /// The attributes a query reads the value of.
    tested: Vec<String>,
    phase: Phase,
    stack: Vec<Open>,
    /// The element whose start tag was read last, when it has no content
    /// and so was not pushed: a void element or a self-closing foreign one.
    childless: Option<(Held, bool)>,
    /// Each query's matches.
    matches: Vec<Vec<Streamed>>,
    /// The matches whose element is open, by query, match and depth.
    open: Vec<(usize, usize, usize)>,
    /// The raw text of the text node being read, and its type.
    text: String,
    text_type: Option<TextType>,
    /// Whether a line feed starting the next text node is dropped, as
    /// after `<pre>`, `<listing>` and `<textarea>`.
    drop_lf: bool,
    /// End tags read, each closing the element on top of the stack.
    end_tags: usize,
    /// What looks like an end tag in text, comments and attribute values.
    accounted: usize,
    doctypes: usize,
}

impl State {
    fn new(queries: &[Query<'_>], tested: Vec<String>) -> Self {
        Self {
            first: queries.iter().map(|query| query.first).collect(),
            tested,
            phase: Phase::BeforeHtml,
            stack: Vec::new(),
            childless: None,
            matches: vec![Vec::new(); queries.len()],
            open: Vec::new(),
            text: String::new(),
            text_type: None,
            drop_lf: false,
            end_tags: 0,
            accounted: 0,
            doctypes: 0,
        }
    }

    /// Ends the text node being read when the chunk next read is of
    /// another type.
    fn flush_if(&mut self, next: TextType) -> HandlerResult {
        if self.text_type.is_some_and(|read| read != next) {
            self.flush()?;
        }
        Ok(())
    }

    /// Ends the text node being read: refused where the tree would move
    /// its text, else appended to every open match.
    fn flush(&mut self) -> HandlerResult {
        let Some(kind) = self.text_type.take() else {
            return Ok(());
        };
        let raw = std::mem::take(&mut self.text);
        self.accounted += end_tags_in(&raw);
        let blank = raw.chars().all(|character| WHITESPACE.contains(&character));
        let top = self.stack.last();
        let placed = match self.phase {
            // Tree construction drops white space before `<head>`.
            Phase::BeforeHtml | Phase::BeforeHead => {
                if !blank {
                    return refused();
                }
                self.drop_lf = false;
                return Ok(());
            }
            Phase::AfterHead | Phase::AfterBody | Phase::AfterHtml => blank,
            Phase::InHead => blank || top.is_some_and(|open| open.held.name != "head"),
            Phase::InBody => {
                blank
                    || !top.is_some_and(|open| {
                        open.html && TABLE_CONTEXT.contains(&open.held.name.as_str())
                    })
            }
        };
        if !placed {
            return refused();
        }
        let mut text = match kind {
            TextType::Data | TextType::RCData => decode(&raw, Place::Text),
            _ => raw.replace("\r\n", "\n").replace('\r', "\n"),
        };
        if std::mem::take(&mut self.drop_lf) && text.starts_with('\n') {
            text.remove(0);
        }
        for &(query, index, _) in &self.open {
            self.matches[query][index].text.push_str(&text);
        }
        Ok(())
    }

    /// Reads the start tag of an element named `name`, with `raw`
    /// attributes, which lol-html reads as `read`: an HTML element or not,
    /// self-closing or not, and with content, so pushed on the stack, or
    /// not.
    fn start(&mut self, name: String, raw: Vec<(String, String)>, read: Read) -> HandlerResult {
        self.flush()?;
        self.drop_lf = false;
        self.childless = None;
        // The element's namespace as the tree reads it: lol-html reads an
        // SVG `<title>` as HTML.
        let html = !["svg", "math"].contains(&name.as_str())
            && self.stack.last().is_none_or(|open| open.html);
        let pushed = read.content;
        if (html && !read.html) || (!html && pushed == read.self_closing) {
            return refused();
        }
        let mut attributes: Vec<(String, String)> = Vec::with_capacity(raw.len());
        for (attribute, value) in raw {
            self.accounted += end_tags_in(&value);
            if self.tested.contains(&attribute) && (value.contains('&') || value.contains('\r')) {
                return refused();
            }
            if attributes.iter().all(|(kept, _)| *kept != attribute) {
                attributes.push((attribute, decode(&value, Place::Attribute)));
            }
        }
        match self.phase {
            Phase::BeforeHtml if name == "html" => self.phase = Phase::BeforeHead,
            Phase::BeforeHead if name == "head" => self.phase = Phase::InHead,
            Phase::AfterHead if name == "body" => self.phase = Phase::InBody,
            Phase::InHead
                if self
                    .stack
                    .last()
                    .is_some_and(|open| open.held.name == "head")
                    && HEAD_CONTENT.contains(&name.as_str()) => {}
            Phase::InBody => self.in_body(&name, html, &attributes)?,
            _ => return refused(),
        }
        self.drop_lf = html && ["pre", "listing", "textarea"].contains(&name.as_str());
        let held = Held { name, attributes };
        if pushed {
            self.stack.push(Open { held, html });
        } else {
            self.childless = Some((held, html));
        }
        Ok(())
    }

    /// Refuses a start tag of `name` in the body that tree construction
    /// would not insert where the stack's top is.
    fn in_body(&self, name: &str, html: bool, attributes: &[(String, String)]) -> HandlerResult {
        let Some(top) = self.stack.last() else {
            return refused();
        };
        if !top.html {
            let font = name == "font"
                && attributes
                    .iter()
                    .any(|(attribute, _)| ["color", "face", "size"].contains(&attribute.as_str()));
            if INTEGRATION_POINTS.contains(&top.held.name.as_str())
                || BREAKOUT.contains(&name)
                || font
            {
                return refused();
            }
            return Ok(());
        }
        let top_name = top.held.name.as_str();
        if REFUSED.contains(&name) && html {
            return refused();
        }
        if TABLE_CONTEXT.contains(&top_name) {
            // SVG or `MathML` in a table is moved before it.
            if !html {
                return refused();
            }
            let allowed: &[&str] = match top_name {
                "table" => &["caption", "colgroup", "tbody", "thead", "tfoot"],
                "tbody" | "thead" | "tfoot" => &["tr"],
                "tr" => &["td", "th"],
                _ => &["col"],
            };
            if !allowed.contains(&name) && !["script", "style"].contains(&name) {
                return refused();
            }
            return Ok(());
        }
        if !html {
            return Ok(());
        }
        if TABLE_PARTS.contains(&name) {
            return refused();
        }
        let open = |named: &str| {
            self.stack
                .iter()
                .any(|open| open.html && open.held.name == named)
        };
        let refuses = match name {
            "a" | "button" | "form" => open(name),
            "li" => self.in_scope(&["li"], &["ol", "ul"]),
            "dd" | "dt" => self.in_scope(&["dd", "dt"], &["dl"]),
            _ if HEADINGS.contains(&name) => HEADINGS.contains(&top_name),
            _ => false,
        };
        if refuses || (P_CLOSERS.contains(&name) && self.in_scope(&["p"], &["button"])) {
            return refused();
        }
        Ok(())
    }

    /// Whether an element named one of `named` is open above the nearest
    /// element of the default scope or one of `bounds`.
    fn in_scope(&self, named: &[&str], bounds: &[&str]) -> bool {
        for open in self.stack.iter().rev() {
            let name = open.held.name.as_str();
            if named.contains(&name) {
                return true;
            }
            if !open.html || SCOPE.contains(&name) || bounds.contains(&name) {
                return false;
            }
        }
        false
    }

    /// Records a match of `query`: the element whose start tag was read
    /// last.
    fn matched(&mut self, query: usize) -> HandlerResult {
        if self.first[query] && !self.matches[query].is_empty() {
            return Ok(());
        }
        let (own, html) = match &self.childless {
            Some((held, html)) => (Some(held.clone()), *html),
            None => (None, self.stack.last().is_some_and(|open| open.html)),
        };
        if !html {
            return refused();
        }
        let pushed = own.is_none();
        let held = own
            .into_iter()
            .chain(self.stack.iter().rev().map(|open| open.held.clone()))
            .collect();
        self.matches[query].push(Streamed {
            held,
            text: String::new(),
        });
        if pushed {
            let index = self.matches[query].len() - 1;
            self.open.push((query, index, self.stack.len() - 1));
        }
        Ok(())
    }

    /// Reads the end tag `name` that lol-html ended the open element
    /// `own`, at `depth`, with.
    fn end(&mut self, (own, depth): (&str, usize), name: &str) -> HandlerResult {
        self.flush()?;
        self.drop_lf = false;
        if name == "body" || name == "html" {
            return self.body_ended(own, name);
        }
        let top = depth + 1 == self.stack.len();
        if own != name || !top || !matches!(self.phase, Phase::InHead | Phase::InBody) {
            return refused();
        }
        if own == "head" {
            self.phase = Phase::AfterHead;
        }
        self.pop();
        self.end_tags += 1;
        Ok(())
    }

    /// Reads `</body>` or `</html>`, `name`, which lol-html ended the open
    /// element `own` with. The tree keeps every element open after it, and
    /// reads white space after it as the text of the element on top.
    fn body_ended(&mut self, own: &str, name: &str) -> HandlerResult {
        let in_body = self
            .stack
            .get(1)
            .is_some_and(|open| open.held.name == "body");
        if !in_body {
            return refused();
        }
        if own != name {
            // An element left open, which lol-html ended with the body.
            return Ok(());
        }
        match (name, self.phase) {
            ("body", Phase::InBody) => self.phase = Phase::AfterBody,
            ("html", Phase::InBody | Phase::AfterBody) => self.phase = Phase::AfterHtml,
            _ => return refused(),
        }
        self.end_tags += 1;
        Ok(())
    }

    /// Pops the stack's top, closing its matches.
    fn pop(&mut self) {
        self.stack.pop();
        let depth = self.stack.len();
        self.open.retain(|&(_, _, at)| at < depth);
    }

    /// The matches, once the document was read whole with `written` end
    /// tags in its text: none when one of them closed no element, as an
    /// end tag that the tree reads as a start tag (`</p>`, `</br>`) or
    /// that closes another element can.
    fn finished(mut self, written: usize) -> Option<Vec<Vec<Streamed>>> {
        self.flush().ok()?;
        (written == self.end_tags + self.accounted).then_some(self.matches)
    }
}

/// Reads `element`'s start tag, keeping an end tag handler for one with
/// content.
fn started(state: &Rc<RefCell<State>>, element: &mut Element<'_, '_>) -> HandlerResult {
    let name = element.tag_name();
    let read = Read {
        html: element.namespace_uri() == "http://www.w3.org/1999/xhtml",
        self_closing: element.is_self_closing(),
        content: element.can_have_content(),
    };
    let pushed = read.content;
    let attributes = element
        .attributes()
        .iter()
        .map(|attribute| (attribute.name(), attribute.value()))
        .collect();
    let depth = {
        let mut state = state.borrow_mut();
        state.start(name.clone(), attributes, read)?;
        state.stack.len().saturating_sub(1)
    };
    if pushed {
        let state = Rc::clone(state);
        element.on_end_tag(Box::new(move |end| {
            state.borrow_mut().end((&name, depth), &end.name())
        }))?;
    }
    Ok(())
}

/// How lol-html read a start tag.
#[derive(Clone, Copy)]
struct Read {
    /// Whether it reads an HTML element.
    html: bool,
    self_closing: bool,
    /// Whether the element has content: an end tag ends it.
    content: bool,
}

/// How many times `text` holds `</` before an ASCII letter: an end tag's
/// start.
fn end_tags_in(text: &str) -> usize {
    text.as_bytes()
        .windows(3)
        .filter(|window| window[0] == b'<' && window[1] == b'/' && window[2].is_ascii_alphabetic())
        .count()
}

/// The attributes `selector`'s text compares the value of, lower-cased:
/// each attribute selector with a value, and `class` and `id` for a `.` and
/// a `#` anywhere.
fn tested(selector: &str) -> Vec<String> {
    let mut tested = Vec::new();
    if selector.contains('.') {
        tested.push("class".to_owned());
    }
    if selector.contains('#') {
        tested.push("id".to_owned());
    }
    for (at, _) in selector.match_indices('[') {
        let inside = &selector[at + 1..];
        let inside = &inside[..inside.find(']').unwrap_or(inside.len())];
        let Some(operator) = inside.find('=') else {
            continue;
        };
        // `ns|name`: the name after its namespace; `|=`'s own `|` is no part
        // of it.
        let named = inside[..operator].trim_end_matches(['~', '|', '^', '$', '*']);
        let name = named.rsplit('|').next().unwrap_or(named).trim();
        tested.push(name.to_ascii_lowercase());
    }
    tested
}

/// The elements a `<head>` holds before `<body>`; any other ends it.
const HEAD_CONTENT: [&str; 9] = [
    "base", "basefont", "bgsound", "link", "meta", "title", "style", "script", "noframes",
];

/// The elements whose parse lol-html does not follow, or that the tree
/// moves, merges, ignores or closes on their own.
const REFUSED: [&str; 20] = [
    "html",
    "head",
    "body",
    "frameset",
    "frame",
    "template",
    "select",
    "option",
    "optgroup",
    "plaintext",
    "image",
    "isindex",
    "noscript",
    "rb",
    "rp",
    "rt",
    "rtc",
    "menuitem",
    "search",
    "nobr",
];

/// The elements whose content tree construction reads in a table mode.
const TABLE_CONTEXT: [&str; 6] = ["table", "tbody", "thead", "tfoot", "tr", "colgroup"];

/// The elements of a table's structure.
const TABLE_PARTS: [&str; 9] = [
    "caption", "col", "colgroup", "tbody", "thead", "tfoot", "tr", "td", "th",
];

/// The elements of the default scope.
const SCOPE: [&str; 9] = [
    "applet", "caption", "html", "table", "td", "th", "marquee", "object", "template",
];

const HEADINGS: [&str; 6] = ["h1", "h2", "h3", "h4", "h5", "h6"];

/// The start tags that close a `<p>` in button scope.
const P_CLOSERS: [&str; 39] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "center",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "header",
    "hgroup",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "section",
    "summary",
    "ul",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "pre",
    "listing",
    "form",
    "table",
    "hr",
    "xmp",
    "li",
    "dd",
    "dt",
];

/// SVG's and `MathML`'s elements whose content is read as HTML.
const INTEGRATION_POINTS: [&str; 9] = [
    "foreignobject",
    "desc",
    "title",
    "mi",
    "mo",
    "mn",
    "ms",
    "mtext",
    "annotation-xml",
];

/// The start tags that end SVG or `MathML` content.
const BREAKOUT: [&str; 44] = [
    "b",
    "big",
    "blockquote",
    "body",
    "br",
    "center",
    "code",
    "dd",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "hr",
    "i",
    "img",
    "li",
    "listing",
    "menu",
    "meta",
    "nobr",
    "ol",
    "p",
    "pre",
    "ruby",
    "s",
    "small",
    "span",
    "strong",
    "strike",
    "sub",
    "sup",
    "table",
    "tt",
    "u",
    "ul",
    "var",
];

#[cfg(test)]
mod tests;
