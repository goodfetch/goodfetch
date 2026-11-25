//! What each document's one hiding sheet holds (ADRs 0047 and 0048): live
//! hiding's rules and the Consent prompt runner's, written into a single
//! style sheet per document, so each frame gets one CSS injection whoever
//! hides in it. Live hiding's part waits, in a document a navigation named,
//! until the runner's first detection pass and wall test there ended, so no
//! cosmetic rule hides a consent wall before it is judged (ADR 0048). Pure:
//! which commands a change needs is decided here, and tested without a
//! browser; [`super::Sheets`] sends them.

use std::collections::{BTreeMap, BTreeSet};

use crate::cdp::TargetSession;

/// The browser's ID of a style sheet.
type StyleSheetId = String;
/// The browser's ID of a frame.
pub(super) type FrameId = String;

/// Who writes a part of a document's sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Part {
    /// Live hiding's rules, from the Session's Filter lists.
    Hiding,
    /// The Consent prompt runner's pre-hiding and hiding rules.
    Consent,
}

/// One frame's document, by its target's session and the frame's ID.
pub(super) type Key = (TargetSession, FrameId);

/// One document's sheet: the sheet the browser made, once made, and each
/// part's text.
#[derive(Debug, Default)]
struct Document {
    /// The loader of the document, when a navigation named it.
    loader: Option<String>,
    sheet: Option<StyleSheetId>,
    hiding: String,
    consent: String,
    /// The selectors of the runner's rules the guard lifted, which its
    /// part no longer writes.
    consent_lifted: BTreeSet<String>,
    /// Whether live hiding's part waits for the runner's first pass.
    held: bool,
    /// What the browser's sheet holds now.
    written: String,
}

impl Document {
    /// The sheet's whole text: each part that holds a rule, live hiding's
    /// first once it no longer waits.
    fn text(&self) -> String {
        let hiding = if self.held { "" } else { self.hiding.as_str() };
        let consent = if self.consent_lifted.is_empty() {
            self.consent.clone()
        } else {
            let kept: String = self
                .consent
                .split_inclusive('}')
                .filter(|rule| !self.consent_lifted.contains(selector_of(rule)))
                .collect();
            kept.trim().to_owned()
        };
        [hiding, consent.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The selector of the rule `rule`, `<selector> { <declarations> }`.
fn selector_of(rule: &str) -> &str {
    rule.split_once('{')
        .map_or(rule, |(selector, _)| selector)
        .trim()
}

/// What one change asks the browser for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Write {
    /// The document.
    pub(super) key: Key,
    /// Whether its target's `DOM` and `CSS` domains are to be enabled first.
    pub(super) enable: bool,
    /// Its sheet, once made; none to make it first.
    pub(super) sheet: Option<StyleSheetId>,
    /// The text to set.
    pub(super) text: String,
}

/// The documents of one page and the sheets that hide in them.
#[derive(Debug, Default)]
pub(super) struct Documents {
    by_key: BTreeMap<Key, Document>,
    /// The targets whose `DOM` and `CSS` domains are on.
    styled: BTreeSet<TargetSession>,
    /// While a click in a frame goes in, that frame's document: every other
    /// document's sheet is empty until it ends.
    lifted: Option<Key>,
    /// The order of the last ordered change sent, by document and part.
    orders: BTreeMap<(Key, Part), u64>,
}

impl Documents {
    /// `session`'s `DOM` and `CSS` domains are on.
    pub(super) fn styled(&mut self, session: &TargetSession) {
        self.styled.insert(session.clone());
    }

    /// `frame` of `session` committed the document of `loader`: a document
    /// other than the one its sheet was made in starts with no sheet and no
    /// rule, live hiding's part waiting for the runner's first pass; the
    /// same document, reported again, keeps all.
    pub(super) fn navigated(&mut self, session: &TargetSession, frame: &FrameId, loader: &str) {
        let key = (session.clone(), frame.clone());
        let document = self.by_key.entry(key).or_default();
        if document.loader.as_deref() != Some(loader) {
            *document = Document {
                loader: Some(loader.to_owned()),
                held: true,
                ..Document::default()
            };
        }
    }

    /// Whether a change of `part` of `key`'s sheet made in `order` goes
    /// out: only when no change of that part made later went out before it.
    pub(super) fn ordered(&mut self, key: &Key, part: Part, order: u64) -> bool {
        let last = self.orders.entry((key.clone(), part)).or_insert(order);
        if order < *last {
            return false;
        }
        *last = order;
        true
    }

    /// `part` of `key`'s sheet becomes `text`; the write it needs, none when
    /// the browser's sheet already holds the whole text or a click in another
    /// frame keeps it empty for now.
    pub(super) fn set(&mut self, key: &Key, part: Part, text: String) -> Option<Write> {
        let document = self.by_key.entry(key.clone()).or_default();
        match part {
            Part::Hiding => document.hiding = text,
            Part::Consent => document.consent = text,
        }
        if self.lifted.as_ref().is_some_and(|lifted| lifted != key) {
            return None;
        }
        self.write(key)
    }

    /// Live hiding's part of `key`'s document no longer waits, the
    /// runner's first pass there having ended, or, with `loader`, the
    /// document of that load having waited long enough; the write it needs,
    /// as [`Self::set`] answers it.
    pub(super) fn release(&mut self, key: &Key, loader: Option<&str>) -> Option<Write> {
        let document = self.by_key.get_mut(key)?;
        if !document.held || loader.is_some_and(|loader| document.loader.as_deref() != Some(loader))
        {
            return None;
        }
        document.held = false;
        if self.lifted.as_ref().is_some_and(|lifted| lifted != key) {
            return None;
        }
        self.write(key)
    }

    /// The selectors of the rules the Consent prompt runner hides prompts
    /// with in `key`'s document: each rule of its part that sets
    /// `display: none` or `opacity: 0`.
    pub(super) fn consent_selectors(&self, key: &Key) -> Vec<String> {
        let Some(document) = self.by_key.get(key) else {
            return Vec::new();
        };
        document
            .consent
            .split('}')
            .filter_map(|rule| rule.split_once('{'))
            .filter(|(_, declarations)| {
                let declarations = declarations.replace(' ', "");
                declarations.contains("display:none") || declarations.contains("opacity:0")
            })
            .map(|(selector, _)| selector.trim().to_owned())
            .filter(|selector| !selector.is_empty())
            .collect()
    }

    /// The selectors of [`Self::consent_selectors`] the guard did not lift.
    pub(super) fn consent_applied(&self, key: &Key) -> Vec<String> {
        let lifted = self
            .by_key
            .get(key)
            .map(|document| &document.consent_lifted);
        self.consent_selectors(key)
            .into_iter()
            .filter(|selector| !lifted.is_some_and(|lifted| lifted.contains(selector)))
            .collect()
    }

    /// The guard found an action's target hidden by the runner's rules of
    /// `selectors` in `key`'s document: they are lifted there until its
    /// next load, whatever the runner sets; the write it needs, as
    /// [`Self::set`] answers it.
    pub(super) fn lift_consent(&mut self, key: &Key, selectors: &[String]) -> Option<Write> {
        let document = self.by_key.get_mut(key)?;
        document.consent_lifted.extend(selectors.iter().cloned());
        if self.lifted.as_ref().is_some_and(|lifted| lifted != key) {
            return None;
        }
        self.write(key)
    }

    /// The write that brings `key`'s sheet to its whole text, if it differs.
    fn write(&mut self, key: &Key) -> Option<Write> {
        let document = self.by_key.get(key)?;
        let text = document.text();
        if text == document.written && (document.sheet.is_some() || text.is_empty()) {
            return None;
        }
        Some(Write {
            key: key.clone(),
            enable: !self.styled.contains(&key.0),
            sheet: document.sheet.clone(),
            text,
        })
    }

    /// `write` went through: `sheet` is its document's sheet, holding its
    /// text.
    pub(super) fn written(&mut self, write: &Write, sheet: StyleSheetId) {
        self.styled.insert(write.key.0.clone());
        if let Some(document) = self.by_key.get_mut(&write.key) {
            document.sheet = Some(sheet);
            document.written.clone_from(&write.text);
        }
    }

    /// `write` failed: its document's sheet is made again on the next one,
    /// as the browser no longer knows it.
    pub(super) fn failed(&mut self, write: &Write) {
        if let Some(document) = self.by_key.get_mut(&write.key) {
            document.sheet = None;
            document.written.clear();
        }
    }

    /// A click in `key`'s frame begins: the writes that empty every other
    /// document's sheet that holds a rule. None for the page's own frame,
    /// whose click no other document's hiding reaches, which `page` names.
    pub(super) fn lift(&mut self, key: &Key, page: &Key) -> Vec<Write> {
        if key == page {
            return Vec::new();
        }
        self.lifted = Some(key.clone());
        self.by_key
            .iter()
            .filter(|(other, document)| *other != key && !document.written.is_empty())
            .filter_map(|(other, document)| {
                Some(Write {
                    key: other.clone(),
                    enable: false,
                    sheet: Some(document.sheet.clone()?),
                    text: String::new(),
                })
            })
            .collect()
    }

    /// The click ends: the writes that give each document its whole text
    /// again, rules set while the click went in included.
    pub(super) fn restore(&mut self) -> Vec<Write> {
        if self.lifted.take().is_none() {
            return Vec::new();
        }
        let keys: Vec<Key> = self.by_key.keys().cloned().collect();
        keys.iter().filter_map(|key| self.write(key)).collect()
    }

    /// The iframe target `session` detached: its documents go, with their
    /// refresh orders.
    pub(super) fn forget_session(&mut self, session: &TargetSession) {
        self.styled.remove(session);
        self.by_key.retain(|(owner, _), _| owner != session);
        self.orders.retain(|((owner, _), _), _| owner != session);
    }

    /// `frame` of `session` detached: its document goes, with its refresh
    /// orders.
    pub(super) fn forget_frame(&mut self, session: &TargetSession, frame: &FrameId) {
        let key = (session.clone(), frame.clone());
        self.by_key.remove(&key);
        self.orders.retain(|(document, _), _| *document != key);
    }
}

#[cfg(test)]
mod tests;
