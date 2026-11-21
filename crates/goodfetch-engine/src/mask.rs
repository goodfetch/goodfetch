//! The run's `Mask` (ADRs 0014, 0016 and 0092, spec #564): every value a
//! Run resolved, replaced by `%name%` wherever the engine returns, sends or
//! keeps text, in its raw form and in Goodfetch's encodings.
//!
//! A value joins its Run's [`Mask`] when it resolves and stays until the
//! Run's last handle drops, held as a `SecretString`, zeroized then. The
//! engine's [`Masks`] holds every live Run's values and is what each writer
//! passes its output through, naming the [`Scope`] the output is in:
//!
//! - [`Scope::Returned`]: what a call returns or an event carries, and so
//!   what a model receives. A value shorter than [`SHORT`] characters, or a
//!   one-time code of any length, is masked here only in a read of the
//!   field it was typed in ([`Masks::apply_typed`]), which a field read
//!   calls once a Secret can be typed (#658), so a price or an identifier
//!   that happens to match it is never corrupted.
//! - [`Scope::Kept`]: what the engine keeps beside its outputs: run state,
//!   the error messages it holds, Plans and the Effect journal. Every value.
//! - [`Scope::KeptOutput`]: logs, the Run trace, HAR files, screenshots
//!   and recordings kept as outputs. Every value, but those of a Run whose
//!   `security.redaction` is off (ADR 0016).
//!
//! The forms masked are the raw value, JSON-escaped, URL-encoded with
//! `%20` and with `+`, URL-encoded as a browser encodes a form
//! (`application/x-www-form-urlencoded`) and as JavaScript's
//! `encodeURIComponent` does, base64 and base64url at each of the three
//! alignments inside a longer encoded run (which covers the Basic
//! `Authorization` and `Proxy-Authorization` headers built from a value),
//! and hex in either case. Each scope is one Aho-Corasick automaton over
//! every form of every value it masks, rebuilt only when a value joins or
//! a Run ends, so masking is linear in the bytes scanned whatever the
//! number of values (ADR 0071).
//!
//! The automata are built from the values' forms, which they hold in their
//! own tables until the next rebuild drops them: that memory is freed, not
//! zeroized.
#![expect(
    clippy::disallowed_types,
    reason = "a log writer masks from any thread and a Run's values leave as its last handle drops, which cannot wait: one RwLock guards the values and their automata, with nothing else under it, so there is no lock order for loom to test (ADR 0060)"
)]

use std::borrow::Cow;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard, TryLockError};

use aho_corasick::{AhoCorasick, MatchKind};
use secrecy::zeroize::Zeroizing;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Serialize, Serializer};
use serde_json::Value;
use subtle::ConstantTimeEq as _;

/// The length, in characters, under which a value is masked only where it
/// was typed and in what the engine keeps (spec #564's shape: fixed, not a
/// setting).
pub const SHORT: usize = 8;

/// Where an output goes, which says which values are masked in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// What a call returns or an event carries: every value of
    /// [`SHORT`] characters or more.
    Returned,
    /// What the engine keeps: run state, error messages, Plans and the
    /// Effect journal. Every value.
    Kept,
    /// Logs, the Run trace, HAR files, screenshots and recordings kept as
    /// outputs: every value of a Run whose `security.redaction` is on.
    KeptOutput,
}

/// What kind of value joins a [`Mask`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// A value resolved from a Secret reference: short when it has fewer
    /// than [`SHORT`] characters.
    Value,
    /// A one-time code, such as a TOTP code or a `code_source`'s: short
    /// whatever its length. Joined once the engine types TOTP codes (#658)
    /// and `code_source` codes (#734).
    OneTimeCode,
}

/// Whether a Run's values are masked in [`Scope::KeptOutput`]: its
/// `security.redaction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Redaction {
    /// On, the default.
    On,
    /// Switched off for the Run.
    Off,
}

/// The engine's masks: every live Run's values, and the automata each
/// [`Scope`] masks with. Clones share them.
#[derive(Clone, Default)]
pub struct Masks(Arc<Shared>);

#[derive(Default)]
struct Shared {
    state: RwLock<State>,
}

#[derive(Default)]
struct State {
    /// The next Run's number.
    next: u64,
    entries: Vec<Entry>,
    /// One automaton per [`Scope`], none where it masks no value.
    returned: Option<Automaton>,
    kept: Option<Automaton>,
    kept_output: Option<Automaton>,
}

/// One value of one Run.
struct Entry {
    run: u64,
    name: String,
    value: SecretString,
    short: bool,
    redaction: Redaction,
}

impl State {
    fn automaton(&self, scope: Scope) -> Option<&Automaton> {
        match scope {
            Scope::Returned => self.returned.as_ref(),
            Scope::Kept => self.kept.as_ref(),
            Scope::KeptOutput => self.kept_output.as_ref(),
        }
    }

    /// Builds each scope's automaton again from the entries.
    fn rebuild(&mut self) {
        self.returned = Automaton::of(self.entries.iter().filter(|entry| !entry.short));
        self.kept = Automaton::of(self.entries.iter());
        self.kept_output = Automaton::of(
            self.entries
                .iter()
                .filter(|entry| entry.redaction == Redaction::On),
        );
    }
}

/// One scope's automaton: every form of every value it masks, and each
/// pattern's replacement.
struct Automaton {
    searcher: AhoCorasick,
    replacements: Vec<Arc<str>>,
}

impl Automaton {
    /// The automaton over every form of `entries`' values, none when they
    /// have none.
    fn of<'e>(entries: impl Iterator<Item = &'e Entry>) -> Option<Self> {
        let mut patterns: Vec<Zeroizing<Vec<u8>>> = Vec::new();
        let mut replacements = Vec::new();
        for entry in entries {
            let replacement: Arc<str> = format!("%{}%", entry.name).into();
            for form in forms(entry.value.expose_secret()) {
                replacements.push(Arc::clone(&replacement));
                patterns.push(form);
            }
        }
        if patterns.is_empty() {
            return None;
        }
        let searcher = AhoCorasick::builder()
            .match_kind(MatchKind::LeftmostLongest)
            .build(patterns.iter().map(|pattern| pattern.as_slice()))
            .expect("invariant: the forms of a run's values fit an automaton");
        Some(Self {
            searcher,
            replacements,
        })
    }

    /// `haystack` with every match replaced, borrowed when none.
    fn replace<'h>(&self, haystack: &'h [u8]) -> Cow<'h, [u8]> {
        let mut found = self.searcher.find_iter(haystack).peekable();
        if found.peek().is_none() {
            return Cow::Borrowed(haystack);
        }
        let mut out = Vec::with_capacity(haystack.len());
        self.replaced(haystack, found, &mut out);
        Cow::Owned(out)
    }

    /// Appends `haystack` to `out` with every match replaced.
    fn replace_into(&self, haystack: &[u8], out: &mut Vec<u8>) {
        self.replaced(haystack, self.searcher.find_iter(haystack), out);
    }

    /// Appends `haystack` to `out` with each of its matches `found`
    /// replaced.
    fn replaced(
        &self,
        haystack: &[u8],
        found: impl Iterator<Item = aho_corasick::Match>,
        out: &mut Vec<u8>,
    ) {
        let mut cursor = 0;
        for found in found {
            out.extend_from_slice(&haystack[cursor..found.start()]);
            out.extend_from_slice(self.replacement(found.pattern()));
            cursor = found.end();
        }
        out.extend_from_slice(&haystack[cursor..]);
    }

    fn replacement(&self, pattern: aho_corasick::PatternID) -> &[u8] {
        self.replacements[pattern.as_usize()].as_bytes()
    }
}

/// Every form of `value` the engine masks, each once: raw, JSON-escaped,
/// URL-encoded with `%20` and with `+`, as a browser's form and
/// `encodeURIComponent` encode it, base64 and base64url at each alignment,
/// and hex in both cases.
fn forms(value: &str) -> Vec<Zeroizing<Vec<u8>>> {
    let bytes = value.as_bytes();
    let mut forms: Vec<Zeroizing<Vec<u8>>> = Vec::new();
    let mut add = |form: Vec<u8>| {
        let form = Zeroizing::new(form);
        if !form.is_empty() && forms.iter().all(|earlier| **earlier != *form) {
            forms.push(form);
        }
    };
    add(bytes.to_vec());
    let quoted = Zeroizing::new(
        serde_json::to_string(value).expect("invariant: a string serializes to JSON"),
    );
    add(quoted.as_bytes()[1..quoted.len() - 1].to_vec());
    let percent = Zeroizing::new(percent_encoded(bytes, RFC_3986_UNRESERVED));
    add(percent.as_bytes().to_vec());
    add(percent.replace("%20", "+").into_bytes());
    let form = Zeroizing::new(percent_encoded(bytes, FORM_URLENCODED_KEPT));
    add(form.replace("%20", "+").into_bytes());
    add(percent_encoded(bytes, URI_COMPONENT_KEPT).into_bytes());
    for alphabet in [
        &data_encoding::BASE64_NOPAD,
        &data_encoding::BASE64URL_NOPAD,
    ] {
        for alignment in 0..3 {
            add(base64_at(alphabet, bytes, alignment));
        }
    }
    add(data_encoding::HEXLOWER.encode(bytes).into_bytes());
    add(data_encoding::HEXUPPER.encode(bytes).into_bytes());
    forms
}

/// The marks RFC 3986 leaves unreserved, which Goodfetch's own URL encoding
/// keeps beside the ASCII letters and digits.
const RFC_3986_UNRESERVED: &[u8] = b"-._~";

/// The marks the WHATWG URL Standard's `application/x-www-form-urlencoded`
/// serializer keeps, as a browser writes a form's body or a GET form's
/// query: `*` kept, `~` escaped, and a space written `+`.
const FORM_URLENCODED_KEPT: &[u8] = b"*-._";

/// The marks JavaScript's `encodeURIComponent` keeps.
const URI_COMPONENT_KEPT: &[u8] = b"-_.!~*'()";

/// `value` percent-encoded, every byte but the ASCII letters, digits and
/// `kept`, in uppercase hex.
fn percent_encoded(value: &[u8], kept: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len() * 3);
    for &byte in value {
        if byte.is_ascii_alphanumeric() || kept.contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0xf)]));
        }
    }
    encoded
}

/// The base64 characters that encode `value` alone when it starts
/// `alignment` bytes into a group of three, as inside a Basic header's
/// `user:password`: the characters shared with the bytes around it are cut.
fn base64_at(alphabet: &data_encoding::Encoding, value: &[u8], alignment: usize) -> Vec<u8> {
    let mut padded = Zeroizing::new(vec![0; alignment]);
    padded.extend_from_slice(value);
    let encoded = Zeroizing::new(alphabet.encode(&padded).into_bytes());
    let start = [0, 2, 3][alignment];
    let end = padded.len() / 3 * 4 + padded.len() % 3;
    if start >= end {
        return Vec::new();
    }
    encoded[start..end].to_vec()
}

impl std::fmt::Debug for Masks {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Masks").finish_non_exhaustive()
    }
}

impl Masks {
    /// No value yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// A new Run's mask, its values masked in [`Scope::KeptOutput`] unless
    /// `redaction` is off. Its values leave once its last handle drops.
    pub fn for_run(&self, redaction: Redaction) -> Mask {
        let run = {
            let mut state = self.write();
            state.next += 1;
            state.next
        };
        Mask(Arc::new(RunMask {
            masks: self.clone(),
            run,
            redaction,
        }))
    }

    /// `text` with every value `scope` masks replaced by its `%name%`.
    pub fn apply<'t>(&self, text: &'t str, scope: Scope) -> Cow<'t, str> {
        utf8(self.apply_bytes(text.as_bytes(), scope), text)
    }

    /// As [`Masks::apply`], without waiting: none while a value joins or a
    /// Run's values leave, as when a panic broke off that change.
    pub fn try_apply<'t>(&self, text: &'t str, scope: Scope) -> Option<Cow<'t, str>> {
        let state = match self.0.state.try_read() {
            Ok(state) => state,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return None,
        };
        let masked = match state
            .automaton(scope)
            .map(|automaton| automaton.replace(text.as_bytes()))
        {
            None | Some(Cow::Borrowed(_)) => Cow::Borrowed(text.as_bytes()),
            Some(Cow::Owned(masked)) => Cow::Owned(masked),
        };
        Some(utf8(masked, text))
    }

    /// `bytes` with every value `scope` masks replaced by its `%name%`.
    pub fn apply_bytes<'t>(&self, bytes: &'t [u8], scope: Scope) -> Cow<'t, [u8]> {
        let state = self.read();
        match state
            .automaton(scope)
            .map(|automaton| automaton.replace(bytes))
        {
            None | Some(Cow::Borrowed(_)) => Cow::Borrowed(bytes),
            Some(Cow::Owned(masked)) => Cow::Owned(masked),
        }
    }

    /// Appends `bytes` to `out` with every value `scope` masks replaced by
    /// its `%name%`, as [`Masks::apply_bytes`] masks them: a writer that
    /// masks line after line reuses `out` rather than taking a new buffer
    /// for each line it masks. Each call reads the values masked at that
    /// moment, so a line masks every value joined before it was written.
    pub fn apply_bytes_into(&self, bytes: &[u8], scope: Scope, out: &mut Vec<u8>) {
        let state = self.read();
        match state.automaton(scope) {
            Some(automaton) => automaton.replace_into(bytes, out),
            None => out.extend_from_slice(bytes),
        }
    }

    /// `text`, a read of a field where the values named `typed` were
    /// typed, with every value [`Scope::Returned`] masks and those values,
    /// however short, replaced by their `%name%`.
    pub fn apply_typed<'t>(&self, text: &'t str, typed: &[&str]) -> Cow<'t, str> {
        let returned = self.apply(text, Scope::Returned);
        let short = {
            let state = self.read();
            Automaton::of(
                state
                    .entries
                    .iter()
                    .filter(|entry| entry.short && typed.contains(&entry.name.as_str())),
            )
        };
        let Some(short) = short else {
            return returned;
        };
        match short.replace(returned.as_bytes()) {
            Cow::Borrowed(_) => returned,
            Cow::Owned(masked) => Cow::Owned(
                String::from_utf8(masked)
                    .expect("invariant: replacing whole UTF-8 forms in UTF-8 text leaves UTF-8"),
            ),
        }
    }

    /// Masks every string of `value`, its members' names included, as
    /// `scope` says.
    pub fn apply_value(&self, value: &mut Value, scope: Scope) {
        match value {
            Value::String(text) => {
                if let Cow::Owned(masked) = self.apply(text, scope) {
                    *text = masked;
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.apply_value(item, scope);
                }
            }
            Value::Object(members) => {
                for (name, mut member) in std::mem::take(members) {
                    self.apply_value(&mut member, scope);
                    members.insert(self.apply(&name, scope).into_owned(), member);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }

    /// `value`'s JSON text with every string of it masked, its members'
    /// names included, as `scope` says: the text of what
    /// [`Masks::apply_value`] leaves, written from `value` as it is, with
    /// no copy of it.
    pub fn json(&self, value: &Value, scope: Scope) -> String {
        let state = self.read();
        match state.automaton(scope) {
            None => value.to_string(),
            Some(automaton) => serde_json::to_string(&MaskedJson { value, automaton })
                .expect("invariant: a JSON value encodes"),
        }
    }

    /// `line`, one JSON message, masked as `scope` says and still one JSON
    /// message: masked as text, or string by string where a value's raw
    /// form spanned the message's own syntax. A line no mask changed comes
    /// back as it came, unread: only a line holding a masked value is read
    /// again, and encoded again only when masking it as text broke it.
    pub fn apply_json_line(&self, line: Vec<u8>, scope: Scope) -> Vec<u8> {
        let masked = match self.apply_bytes(&line, scope) {
            Cow::Borrowed(_) => return line,
            Cow::Owned(masked) => masked,
        };
        if serde_json::from_slice::<serde::de::IgnoredAny>(&masked).is_ok() {
            return masked;
        }
        let Ok(mut value) = serde_json::from_slice::<Value>(&line) else {
            return masked;
        };
        self.apply_value(&mut value, scope);
        let mut rewritten = serde_json::to_vec(&value).expect("invariant: a JSON value encodes");
        if line.ends_with(b"\n") {
            rewritten.push(b'\n');
        }
        rewritten
    }

    /// Adds `value` under `name` for `run`, and rebuilds the automata,
    /// unless `run` holds it under `name` already: whether it joined.
    fn join(
        &self,
        run: u64,
        name: &str,
        value: &SecretString,
        reach: Reach,
        redaction: Redaction,
    ) -> bool {
        let short = reach == Reach::OneTimeCode || value.expose_secret().chars().count() < SHORT;
        let mut state = self.write();
        let held = state.entries.iter().any(|entry| {
            entry.run == run
                && entry.name == name
                && entry.short == short
                && bool::from(
                    entry
                        .value
                        .expose_secret()
                        .as_bytes()
                        .ct_eq(value.expose_secret().as_bytes()),
                )
        });
        if held {
            return false;
        }
        state.entries.push(Entry {
            run,
            name: name.to_owned(),
            value: value.clone(),
            short,
            redaction,
        });
        state.rebuild();
        true
    }

    fn leave(&self, run: u64) {
        let mut state = self.write();
        let before = state.entries.len();
        state.entries.retain(|entry| entry.run != run);
        if state.entries.len() != before {
            state.rebuild();
        }
    }

    fn read(&self) -> RwLockReadGuard<'_, State> {
        self.0.state.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, State> {
        self.0.state.write().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One Run's mask: the values it resolved join it. Clones share it; its
/// values leave the engine's [`Masks`] once the last clone drops.
#[derive(Clone)]
pub struct Mask(Arc<RunMask>);

struct RunMask {
    masks: Masks,
    run: u64,
    redaction: Redaction,
}

impl Drop for RunMask {
    fn drop(&mut self) {
        self.masks.leave(self.run);
    }
}

impl std::fmt::Debug for Mask {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Mask")
            .field("redaction", &self.0.redaction)
            .finish_non_exhaustive()
    }
}

impl Mask {
    /// Adds `value` under `name`, a Secret placeholder's name or the
    /// setting's key where no placeholder names it, for the rest of the
    /// Run: masked as `%name%`. A value the Run holds under `name` already
    /// does not join again, so the masks are rebuilt only when a new value
    /// joins; whether it joined.
    pub fn join(&self, name: &str, value: &SecretString, reach: Reach) -> bool {
        self.0
            .masks
            .join(self.0.run, name, value, reach, self.0.redaction)
    }

    /// The engine's masks this one is part of.
    pub fn masks(&self) -> &Masks {
        &self.0.masks
    }

    /// Whether the Run's values are masked in its kept outputs.
    pub fn redaction(&self) -> Redaction {
        self.0.redaction
    }
}

/// A JSON value as [`Masks::json`] writes it: each string through
/// `automaton`, and an object's members under their masked names, as the
/// object [`Masks::apply_value`] leaves keeps them.
struct MaskedJson<'a> {
    value: &'a Value,
    automaton: &'a Automaton,
}

impl MaskedJson<'_> {
    fn masked<'t>(&self, text: &'t str) -> Cow<'t, str> {
        utf8(self.automaton.replace(text.as_bytes()), text)
    }
}

impl Serialize for MaskedJson<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let Self { value, automaton } = *self;
        let within = |value| Self { value, automaton };
        match value {
            Value::String(text) => serializer.serialize_str(&self.masked(text)),
            Value::Array(items) => serializer.collect_seq(items.iter().map(within)),
            Value::Object(members) => {
                let mut named: Vec<(Cow<'_, str>, &Value)> = members
                    .iter()
                    .map(|(name, member)| (self.masked(name), member))
                    .collect();
                if named.iter().any(|(name, _)| matches!(name, Cow::Owned(_))) {
                    // A JSON object keeps its members by name, and of two
                    // given one name the later: the sort is stable.
                    named.sort_by(|(name, _), (other, _)| name.cmp(other));
                    named.dedup_by(|(name, member), (earlier, kept)| {
                        let same = name == earlier;
                        if same {
                            *kept = *member;
                        }
                        same
                    });
                }
                serializer.collect_map(
                    named
                        .iter()
                        .map(|(name, member)| (name.as_ref(), within(*member))),
                )
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => value.serialize(serializer),
        }
    }
}

/// `masked`, the bytes of `text` with whole forms replaced, as text.
fn utf8<'t>(masked: Cow<'t, [u8]>, text: &'t str) -> Cow<'t, str> {
    match masked {
        Cow::Borrowed(_) => Cow::Borrowed(text),
        Cow::Owned(masked) => Cow::Owned(
            String::from_utf8(masked)
                .expect("invariant: replacing whole UTF-8 forms in UTF-8 text leaves UTF-8"),
        ),
    }
}

#[cfg(test)]
mod tests;
