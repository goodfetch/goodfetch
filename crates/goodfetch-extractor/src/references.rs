//! A document's links, images and media, read in one streaming pass with
//! lol-html, no tree built (ADR 0007): each `href` or `src` decoded and
//! resolved against the document's base URL by the WHATWG URL Standard
//! (the `url` crate, ADR 0120's parser), a value that does not parse kept
//! as written with no URL. The pass reads the document as it stands, so a
//! shadow root a browser serialized as a `<template shadowrootmode>` is
//! read with its host's page, and no Clean-up rule reaches it.

use std::cell::RefCell;
use std::rc::Rc;

use goodfetch_protocol::page::{Image, Link, Media, MediaKind};
use lol_html::html_content::{Element, TextChunk, TextType};
use lol_html::{AsciiCompatibleEncoding, HandlerResult, HtmlRewriter, Settings, element, text};
use url::Url;

use crate::character_references::{Place, decode};

/// What the pass reads.
#[derive(Debug, Default)]
pub(crate) struct References {
    pub(crate) links: Vec<Link>,
    pub(crate) images: Vec<Image>,
    pub(crate) media: Vec<Media>,
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
    }));
    let handler = |read: fn(&Shared, &mut Element<'_, '_>) -> HandlerResult| {
        let state = Rc::clone(&state);
        move |element: &mut Element<'_, '_>| read(&state, element)
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
        .append_element_content_handler(element!("a[href], area[href]", handler(read_link)))
        .append_element_content_handler(text!("a[href]", link_text))
        .append_element_content_handler(element!(
            "img[src]",
            handler(|state, element| {
                read_image(state, element);
                Ok(())
            })
        ))
        .append_element_content_handler(element!(
            "video[src], video source[src]",
            handler(|state, element| {
                read_media(state, element, MediaKind::Video);
                Ok(())
            })
        ))
        .append_element_content_handler(element!(
            "audio[src], audio source[src]",
            handler(|state, element| {
                read_media(state, element, MediaKind::Audio);
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
    }
}

/// Starts reading the link `element` is.
fn read_link(state: &Shared, element: &mut Element<'_, '_>) -> HandlerResult {
    let href = attribute(element, "href").unwrap_or_default();
    let rel = attribute(element, "rel")
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
fn read_image(state: &Shared, element: &Element<'_, '_>) {
    let src = attribute(element, "src").unwrap_or_default();
    let mut state = state.borrow_mut();
    let url = state.resolve(&src);
    state.images.push(Image {
        url,
        src,
        alt: attribute(element, "alt"),
    });
}

/// Reads the media source `element` is, of `kind`.
fn read_media(state: &Shared, element: &Element<'_, '_>, kind: MediaKind) {
    let src = attribute(element, "src").unwrap_or_default();
    let mut state = state.borrow_mut();
    let url = state.resolve(&src);
    state.media.push(Media {
        url,
        src,
        kind,
        media_type: attribute(element, "type"),
    });
}

/// The link `reading` read, its text decoded and its white space collapsed.
fn finished(mut reading: Reading) -> Link {
    reading.flush();
    let text = reading
        .link
        .text
        .split_ascii_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Link {
        text,
        ..reading.link
    }
}

/// The value of `element`'s attribute `name`, decoded.
fn attribute(element: &Element<'_, '_>, name: &str) -> Option<String> {
    element
        .get_attribute(name)
        .map(|value| decode(&value, Place::Attribute))
}
