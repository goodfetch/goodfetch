//! A document's links, images, media and head metadata (its title, meta
//! tags, canonical link and language), read in one streaming pass with
//! lol-html, no tree built (ADR 0007): each `href` or `src` decoded and
//! resolved against the document's base URL by the WHATWG URL Standard
//! (the `url` crate, ADR 0120's parser), a value that does not parse kept
//! as written with no URL. The pass reads the HTML it is given as it
//! stands, so a shadow root a browser serialized as a
//! `<template shadowrootmode>` is read with its host's page. The images
//! and media of a page the Clean-up rules changed are read from its tree
//! instead, as the pass would read its serialization (ADR 0138).

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use dom_query::NodeRef;
use goodfetch_counted_work::ParsedHtml;
use goodfetch_protocol::page::{Image, Link, Media, MediaKind, MetaTag};
use lol_html::html_content::{Element, TextChunk, TextType};
use lol_html::{
    AsciiCompatibleEncoding, HandlerResult, HtmlRewriter, Settings, doc_text, element, text,
};
use url::Url;

use crate::canonical::is_canonical_link;
use crate::character_references::{Place, decode};
use crate::whitespace::collapsed;

/// What the pass reads.
#[derive(Debug, Default)]
pub(crate) struct References {
    pub(crate) links: Vec<Link>,
    pub(crate) images: Vec<Image>,
    pub(crate) media: Vec<Media>,
    pub(crate) head: Head,
}

/// The document's meta tags and the head's other metadata, each value
/// decoded and as written.
#[derive(Debug, Default)]
pub(crate) struct Head {
    /// The first `<title>`'s text, white space collapsed.
    pub(crate) title: Option<String>,
    /// The first `<meta name="description">`'s content.
    pub(crate) description: Option<String>,
    /// The first `<link rel="canonical">`'s `href`, when the head holds that
    /// link, not resolved.
    pub(crate) canonical: Option<String>,
    /// The root element's `lang`.
    pub(crate) lang: Option<String>,
    pub(crate) open_graph: Vec<MetaTag>,
    pub(crate) twitter: Vec<MetaTag>,
    /// The other `<meta name>`s, by name in lower case, the first kept.
    pub(crate) meta: BTreeMap<String, String>,
}

/// A link being read: its text so far, and the raw text not decoded yet.
struct Reading {
    link: Link,
    raw: String,
}

impl Reading {
    /// Decodes the raw text read so far onto the link's text.
    fn flush(&mut self) {
        self.link.text.push_str(&decode(&self.raw, Place::Text));
        self.raw.clear();
    }
}

/// The pass's state, which every handler shares.
struct State {
    /// The base URL, when it parses.
    base: Option<Url>,
    /// Every link in document order, read or being read.
    links: Vec<Reading>,
    /// The links whose end tag has not come yet, innermost last.
    open: Vec<usize>,
    images: Vec<Image>,
    media: Vec<Media>,
    head: Head,
    /// The first `<title>`'s raw text while it is read.
    title: Title,
    /// Whether the root element was read.
    root_read: bool,
    /// Whether the first canonical link came: it alone decides, as nothing
    /// after a body's link goes back in the head.
    canonical_read: bool,
    /// Whether the `<link>` about to be read is a `<template>`'s, which a
    /// parser keeps out of the head.
    link_in_template: bool,
}

/// Where the reading of the first `<title>` stands.
enum Title {
    /// No `<title>` came yet.
    Ahead,
    /// The first is being read: its raw text so far.
    Reading(String),
    /// The first was read.
    Read,
}

impl State {
    /// `value` resolved against the base URL; `None` when either does not
    /// parse.
    fn resolve(&self, value: &str) -> Option<String> {
        self.base
            .as_ref()
            .and_then(|base| base.join(value).ok())
            .map(String::from)
    }
}

type Shared = Rc<RefCell<State>>;

/// The links, images and media of `html`, whose base URL is `base_url`,
/// in document order.
pub(crate) fn references(html: &str, base_url: &str) -> References {
    let state: Shared = Rc::new(RefCell::new(State {
        base: Url::parse(base_url).ok(),
        links: Vec::new(),
        open: Vec::new(),
        images: Vec::new(),
        media: Vec::new(),
        head: Head::default(),
        title: Title::Ahead,
        root_read: false,
        canonical_read: false,
        link_in_template: false,
    }));
    let handler = |read: fn(&Shared, &str, &mut Element<'_, '_>) -> HandlerResult| {
        let state = Rc::clone(&state);
        move |element: &mut Element<'_, '_>| read(&state, html, element)
    };
    let title_text = {
        let state = Rc::clone(&state);
        move |chunk: &mut TextChunk<'_>| {
            if let Title::Reading(raw) = &mut state.borrow_mut().title {
                raw.push_str(chunk.as_str());
            }
            Ok(())
        }
    };
    let link_text = {
        let state = Rc::clone(&state);
        move |chunk: &mut TextChunk<'_>| {
            read_link_text(&state, chunk);
            Ok(())
        }
    };
    let settings = Settings::new()
        // Ambiguous markup is read as a tokenizer reads it, never refused.
        .with_strict(false)
        // The document is decoded already: a `<meta charset>` changes nothing.
        .with_adjust_charset_on_meta_tag(false)
        .with_encoding(AsciiCompatibleEncoding::utf_8())
        .append_element_content_handler(element!(
            "html",
            handler(|state, html, element| {
                read_root(state, html, element);
                Ok(())
            })
        ))
        .append_element_content_handler(element!("title", handler(read_title)))
        .append_element_content_handler(text!("title", title_text))
        .append_element_content_handler(element!(
            "meta[content]",
            handler(|state, html, element| {
                read_meta(state, html, element);
                Ok(())
            })
        ))
        // Runs ahead of the next, on the same element.
        .append_element_content_handler(element!("template link[rel][href]", handler(in_template)))
        .append_element_content_handler(element!(
            "link[rel][href]",
            handler(|state, html, element| {
                read_canonical(state, html, element);
                Ok(())
            })
        ))
        .append_element_content_handler(element!("a[href], area[href]", handler(read_link)))
        .append_element_content_handler(text!("a[href]", link_text))
        .append_element_content_handler(element!(
            "img[src]",
            handler(|state, html, element| {
                read_image(state, html, element);
                Ok(())
            })
        ))
        .append_element_content_handler(element!(
            "video[src], video source[src]",
            handler(|state, html, element| {
                read_media(state, html, element, MediaKind::Video);
                Ok(())
            })
        ))
        .append_element_content_handler(element!(
            "audio[src], audio source[src]",
            handler(|state, html, element| {
                read_media(state, html, element, MediaKind::Audio);
                Ok(())
            })
        ));
    let mut rewriter = HtmlRewriter::new(settings, |_: &[u8]| {});
    // Neither call fails: no handler fails, strict mode is off and memory
    // is not capped. Were one to fail, what was read until then stands.
    let _read = rewriter
        .write(html.as_bytes())
        .and_then(|()| rewriter.end());
    let state = Rc::try_unwrap(state)
        .ok()
        .expect("invariant: the handlers holding the state ended with the rewriter")
        .into_inner();
    References {
        links: state.links.into_iter().map(finished).collect(),
        images: state.images,
        media: state.media,
        head: state.head,
    }
}

/// Reads the root element's `lang`, the first `<html>` being the root.
fn read_root(state: &Shared, html: &str, element: &Element<'_, '_>) {
    let mut state = state.borrow_mut();
    if !state.root_read {
        state.root_read = true;
        state.head.lang = attribute(html, element, "lang");
    }
}

/// Starts reading the `<title>` `element` is, when it is the first.
fn read_title(state: &Shared, _html: &str, element: &mut Element<'_, '_>) -> HandlerResult {
    {
        let mut state = state.borrow_mut();
        if !matches!(state.title, Title::Ahead) {
            return Ok(());
        }
        state.title = Title::Reading(String::new());
    }
    let state = Rc::clone(state);
    element.on_end_tag(Box::new(move |_| {
        let mut state = state.borrow_mut();
        if let Title::Reading(raw) = std::mem::replace(&mut state.title, Title::Read) {
            state.head.title = Some(collapsed(&decode(&raw, Place::Text)));
        }
        Ok(())
    }))?;
    Ok(())
}

/// Reads the `<meta>` `element` is: an Open Graph or Twitter tag, or a
/// named one.
fn read_meta(state: &Shared, html: &str, element: &Element<'_, '_>) {
    let Some(content) = attribute(html, element, "content") else {
        return;
    };
    let property = attribute(html, element, "property");
    let name = attribute(html, element, "name");
    let mut state = state.borrow_mut();
    let head = &mut state.head;
    let Some(key) = property.clone().or_else(|| name.clone()) else {
        return;
    };
    if starts_with_ignoring_case(&key, "twitter:") {
        head.twitter.push(MetaTag {
            property: key,
            content,
        });
    } else if property.is_some() || starts_with_ignoring_case(&key, "og:") {
        head.open_graph.push(MetaTag {
            property: key,
            content,
        });
    } else {
        let name = key.to_ascii_lowercase();
        if name == "description" && head.description.is_none() {
            head.description = Some(content.clone());
        }
        head.meta.entry(name).or_insert(content);
    }
}

/// Notes that the `<link>` about to be read is a `<template>`'s.
#[expect(
    clippy::unnecessary_wraps,
    reason = "every element handler of the pass answers a `HandlerResult`"
)]
fn in_template(state: &Shared, _html: &str, _element: &mut Element<'_, '_>) -> HandlerResult {
    state.borrow_mut().link_in_template = true;
    Ok(())
}

/// Reads the `<link>` `element` is, when it is the first canonical one and
/// the head stands where it is.
fn read_canonical(state: &Shared, html: &str, element: &Element<'_, '_>) {
    let mut state = state.borrow_mut();
    if std::mem::take(&mut state.link_in_template)
        || state.canonical_read
        || !attribute(html, element, "rel").is_some_and(|rel| is_canonical_link(&rel))
    {
        return;
    }
    state.canonical_read = true;
    if html
        .get(..element.source_location().bytes().start)
        .is_some_and(head_stands_past)
    {
        state.head.canonical = attribute(html, element, "href");
    }
}

/// The elements a head holds, whether its tag came or closed: the HTML
/// Standard's parser puts each in the head, or ignores its tag, up to the
/// body.
const HEAD_ELEMENTS: [&str; 13] = [
    "html", "head", "base", "basefont", "bgsound", "link", "meta", "title", "noscript", "noframes",
    "style", "script", "template",
];

/// Whether the HTML Standard's parser still holds the head open past
/// `before`, a document up to one of its tags: the start tag of an element
/// no head holds ends it, and so does text other than white space, but for
/// a `<template>`'s, which it keeps to itself. The pass runs once, on what
/// precedes a page's first canonical link, so the rest of the page's tags
/// and text are never read for it. A stray `</body>`, `</html>` or `</br>`
/// also ends the head, which no handler of a tokenizer's sees.
fn head_stands_past(before: &str) -> bool {
    let ended = Rc::new(Cell::new(false));
    let templates = Rc::new(Cell::new(0_usize));
    let settings = Settings::new()
        .with_strict(false)
        .with_adjust_charset_on_meta_tag(false)
        .with_encoding(AsciiCompatibleEncoding::utf_8())
        .append_element_content_handler(element!("*", {
            let (ended, templates) = (Rc::clone(&ended), Rc::clone(&templates));
            move |element: &mut Element<'_, '_>| {
                let tag = element.tag_name();
                if tag == "template" {
                    templates.set(templates.get() + 1);
                    let templates = Rc::clone(&templates);
                    element.on_end_tag(Box::new(move |_| {
                        templates.set(templates.get() - 1);
                        Ok(())
                    }))?;
                } else if templates.get() == 0 && !HEAD_ELEMENTS.contains(&tag.as_str()) {
                    ended.set(true);
                }
                Ok(())
            }
        }))
        .append_document_content_handler(doc_text!({
            let (ended, templates) = (Rc::clone(&ended), Rc::clone(&templates));
            move |chunk| {
                if templates.get() == 0
                    && matches!(chunk.text_type(), TextType::Data)
                    && !chunk
                        .as_str()
                        .bytes()
                        .all(|byte| byte.is_ascii_whitespace())
                {
                    ended.set(true);
                }
                Ok(())
            }
        }));
    let mut rewriter = HtmlRewriter::new(settings, |_: &[u8]| {});
    // Neither call fails, as in `references`.
    let _read = rewriter
        .write(before.as_bytes())
        .and_then(|()| rewriter.end());
    !ended.get()
}

/// Whether `text` starts with `prefix`, ASCII case aside.
fn starts_with_ignoring_case(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
}

/// Starts reading the link `element` is.
fn read_link(state: &Shared, html: &str, element: &mut Element<'_, '_>) -> HandlerResult {
    let href = attribute(html, element, "href").unwrap_or_default();
    let rel = attribute(html, element, "rel")
        .map(|rel| {
            rel.split_ascii_whitespace()
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let index = {
        let mut state = state.borrow_mut();
        let url = state.resolve(&href);
        state.links.push(Reading {
            link: Link {
                url,
                href,
                text: String::new(),
                rel,
            },
            raw: String::new(),
        });
        state.links.len() - 1
    };
    // An `<area>` is void: it has no text and no end tag.
    if element.end_tag_handlers().is_some() {
        state.borrow_mut().open.push(index);
        let state = Rc::clone(state);
        element.on_end_tag(Box::new(move |_| {
            let mut state = state.borrow_mut();
            if let Some(at) = state.open.iter().rposition(|&open| open == index) {
                state.open.remove(at);
            }
            state.links[index].flush();
            Ok(())
        }))?;
    }
    Ok(())
}

/// Adds `chunk` to the text of the innermost link being read.
fn read_link_text(state: &Shared, chunk: &TextChunk<'_>) {
    let mut state = state.borrow_mut();
    let Some(&index) = state.open.last() else {
        return;
    };
    let reading = &mut state.links[index];
    match chunk.text_type() {
        TextType::Data | TextType::RCData => reading.raw.push_str(chunk.as_str()),
        _ => {
            reading.flush();
            reading.link.text.push_str(chunk.as_str());
        }
    }
    // A reference never spans two text nodes.
    if chunk.last_in_text_node() {
        reading.flush();
    }
}

/// Reads the image `element` is.
fn read_image(state: &Shared, html: &str, element: &Element<'_, '_>) {
    let src = attribute(html, element, "src").unwrap_or_default();
    let mut state = state.borrow_mut();
    let url = state.resolve(&src);
    state.images.push(Image {
        url,
        src,
        alt: attribute(html, element, "alt"),
    });
}

/// Reads the media source `element` is, of `kind`.
fn read_media(state: &Shared, html: &str, element: &Element<'_, '_>, kind: MediaKind) {
    let src = attribute(html, element, "src").unwrap_or_default();
    let mut state = state.borrow_mut();
    let url = state.resolve(&src);
    state.media.push(Media {
        url,
        src,
        kind,
        media_type: attribute(html, element, "type"),
    });
}

/// The link `reading` read, its text decoded and its white space collapsed.
fn finished(mut reading: Reading) -> Link {
    reading.flush();
    let text = collapsed(&reading.link.text);
    Link {
        text,
        ..reading.link
    }
}

/// The value of `element`'s attribute `name` in `html`, decoded. It is
/// read from the document's own text where lol-html locates it: lol-html's
/// own value is decoded alone, which drops a byte order mark it begins
/// with.
fn attribute(html: &str, element: &Element<'_, '_>, name: &str) -> Option<String> {
    let attribute = element
        .attributes()
        .iter()
        .find(|attribute| attribute.name() == name)?;
    let value = attribute
        .value_source_location()
        .and_then(|location| html.get(location.bytes()))
        .map_or_else(|| attribute.value(), ToOwned::to_owned);
    Some(decode(&value, Place::Attribute))
}

/// The images and media of `tree`, whose base URL is `base_url`, in
/// document order, as [`references`] reads them from its serialization: a
/// `<template>`'s contents in its place, an `<img src>` an image, and a
/// `<video src>` or `<audio src>`, or a `<source src>` inside either, a
/// medium of its kind.
pub(crate) fn assets(tree: &ParsedHtml, base_url: &str) -> (Vec<Image>, Vec<Media>) {
    let base = Url::parse(base_url).ok();
    let resolve = |value: &str| {
        base.as_ref()
            .and_then(|base| base.join(value).ok())
            .map(String::from)
    };
    let (mut images, mut media) = (Vec::new(), Vec::new());
    // Depth first, by hand, as a page's tree can be deeper than a thread's
    // stack allows a recursion: each element with the `<video>` and
    // `<audio>` elements open around it.
    let mut next: Vec<Visit<'_>> = tree.root().children_it(true).map(Visit::Enter).collect();
    let mut open = Open::default();
    while let Some(visit) = next.pop() {
        let node = match visit {
            Visit::Enter(node) => node,
            Visit::Leave(kind) => {
                open.leave(kind);
                continue;
            }
        };
        if !node.is_element() {
            continue;
        }
        let name = node
            .node_name()
            .map(|name| name.to_string())
            .unwrap_or_default();
        let src = node.attr("src").map(|src| src.to_string());
        if let Some(src) = &src {
            let source = name == "source";
            if name == "img" {
                images.push(Image {
                    url: resolve(src),
                    src: src.clone(),
                    alt: node.attr("alt").map(|alt| alt.to_string()),
                });
            }
            for (kind, element, inside) in [
                (MediaKind::Video, "video", open.videos > 0),
                (MediaKind::Audio, "audio", open.audios > 0),
            ] {
                if name == element || (source && inside) {
                    media.push(Media {
                        url: resolve(src),
                        src: src.clone(),
                        kind,
                        media_type: node.attr("type").map(|kind| kind.to_string()),
                    });
                }
            }
        }
        let kind = match name.as_str() {
            "video" => Some(MediaKind::Video),
            "audio" => Some(MediaKind::Audio),
            _ => None,
        };
        if let Some(kind) = kind {
            open.enter(kind);
            next.push(Visit::Leave(kind));
        }
        next.extend(children(&node).into_iter().rev().map(Visit::Enter));
    }
    (images, media)
}

/// A step of [`assets`]' walk.
enum Visit<'a> {
    /// Into a node.
    Enter(NodeRef<'a>),
    /// Out of a `<video>` or an `<audio>`.
    Leave(MediaKind),
}

/// The `<video>` and `<audio>` elements open around the walk.
#[derive(Default)]
struct Open {
    videos: usize,
    audios: usize,
}

impl Open {
    fn enter(&mut self, kind: MediaKind) {
        match kind {
            MediaKind::Video => self.videos += 1,
            MediaKind::Audio => self.audios += 1,
        }
    }

    fn leave(&mut self, kind: MediaKind) {
        match kind {
            MediaKind::Video => self.videos -= 1,
            MediaKind::Audio => self.audios -= 1,
        }
    }
}

/// The children of `node` in order, a `<template>`'s being its contents',
/// as its serialization writes them.
fn children<'a>(node: &NodeRef<'a>) -> Vec<NodeRef<'a>> {
    let contents = node
        .query(|tree_node| {
            tree_node
                .as_element()
                .and_then(|element| element.template_contents)
        })
        .flatten();
    let mut children: Vec<NodeRef<'a>> = node.children_it(false).collect();
    if let Some(contents) = contents.and_then(|id| node.tree.get(&id)) {
        children.extend(contents.children_it(false));
    }
    children
}
