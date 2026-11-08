//! The contacts a document shows (spec #580, ticket #787), read from the
//! page's tree as a person sees it (ADR 0138): e-mail
//! addresses from `mailto:` links and from text matching the HTML
//! Standard's valid e-mail address syntax; telephone numbers from `tel:`
//! links and from text, with E.164 only when the number carries its
//! country code or the document's language tag carries a region (G1: no
//! country guessed), by libphonenumber's metadata; and social profiles
//! from links to [`crate::social`]'s list. Each value once, where it first
//! appears; a `mailto:` or `tel:` link's own text is not read again, and
//! no text of a script, style, template or the head is read.

use std::collections::HashSet;

use dom_query::NodeRef;
use goodfetch_counted_work::ParsedHtml;
use goodfetch_protocol::page::{Contacts, Email, Phone, SocialProfile};
use phonenumber::country::Id;
use phonenumber::{Mode, PhoneNumber};
use url::Url;

use crate::social;
use crate::whitespace::collapsed;

/// The elements whose text the page does not show.
const UNSHOWN: &[&str] = &[
    "head", "script", "style", "template", "noscript", "textarea", "title",
];

/// The digits a telephone number holds, at least and at most (E.164).
const DIGITS: std::ops::RangeInclusive<usize> = 7..=15;

/// The contacts read so far, each value once.
struct Found {
    contacts: Contacts,
    emails: HashSet<String>,
    phones: HashSet<String>,
    social: HashSet<String>,
}

impl Found {
    fn email(&mut self, value: &str, text: &str) {
        if self.emails.insert(value.to_owned()) {
            self.contacts.emails.push(Email {
                value: value.to_owned(),
                text: text.to_owned(),
            });
        }
    }

    fn phone(&mut self, value: &str, text: &str, e164: Option<String>) {
        if self.phones.insert(value.to_owned()) {
            self.contacts.phones.push(Phone {
                value: value.to_owned(),
                text: text.to_owned(),
                e164,
            });
        }
    }
}

impl Default for Found {
    fn default() -> Self {
        Self {
            contacts: Contacts {
                emails: Vec::new(),
                phones: Vec::new(),
                social: Vec::new(),
            },
            emails: HashSet::new(),
            phones: HashSet::new(),
            social: HashSet::new(),
        }
    }
}

/// The contacts of `tree`, whose base URL is `base_url`, in document order.
pub(crate) fn contacts(tree: &ParsedHtml, base_url: &str) -> Contacts {
    let base = Url::parse(base_url).ok();
    let region = region(tree.html_root().attr("lang").as_deref());
    let mut found = Found::default();
    // Depth first, by hand: a page's tree can be deeper than a thread's
    // stack allows a recursion. An element whose text is not read is not
    // walked into.
    let mut next: Vec<NodeRef<'_>> = tree.root().children_it(true).collect();
    while let Some(node) = next.pop() {
        if node.is_text() {
            read_text(&node.text(), region, &mut found);
            continue;
        }
        if !node.is_element() {
            continue;
        }
        let name = node
            .node_name()
            .map(|name| name.to_string())
            .unwrap_or_default();
        if UNSHOWN.contains(&name.as_str()) {
            continue;
        }
        if matches!(name.as_str(), "a" | "area")
            && let Some(href) = node.attr("href")
            && read_link(&node, &href, base.as_ref(), region, &mut found)
        {
            continue;
        }
        next.extend(node.children_it(true));
    }
    found.contacts
}

/// Reads the link `link` to `href`; whether it was a `mailto:` or `tel:`
/// link, whose text is then not read again.
fn read_link(
    link: &NodeRef<'_>,
    href: &str,
    base: Option<&Url>,
    region: Option<Id>,
    found: &mut Found,
) -> bool {
    let text = || collapsed(&link.text());
    let href = href.trim_matches(|character: char| character.is_ascii_whitespace());
    if let Some(addresses) = without_scheme(href, "mailto:") {
        let addresses = addresses.split('?').next().unwrap_or_default();
        let text = text();
        for address in percent_decoded(addresses).split(',') {
            let address = address.trim();
            if is_email(address) {
                found.email(address, &text);
            }
        }
        return true;
    }
    if let Some(number) = without_scheme(href, "tel:") {
        let number = percent_decoded(number.split(';').next().unwrap_or_default());
        let number = number.trim();
        if number.chars().any(|character| character.is_ascii_digit()) {
            found.phone(number, &text(), e164(number, region));
        }
        return true;
    }
    if let Some(url) = base.and_then(|base| base.join(href).ok()) {
        let rel_me = link.attr("rel").is_some_and(|rel| {
            rel.split_ascii_whitespace()
                .any(|token| token.eq_ignore_ascii_case("me"))
        });
        if let Some(network) = social::profile(&url, rel_me) {
            let value = String::from(url);
            if found.social.insert(value.clone()) {
                found.contacts.social.push(SocialProfile {
                    value,
                    text: text(),
                    network,
                });
            }
        }
    }
    false
}

/// Reads the e-mail addresses and telephone numbers `text` writes. A span
/// written as another value is no number (O27). Otherwise, a number with
/// its country code, or national in the document's region, is one when
/// libphonenumber finds it valid, and gets its E.164; with neither, no
/// country is guessed, so the span is one, with no E.164.
fn read_text(text: &str, region: Option<Id>, found: &mut Found) {
    for address in emails(text) {
        found.email(address, address);
    }
    for (number, after) in numbers(text) {
        if written_as_another_value(number, after) {
            continue;
        }
        if number.starts_with('+') || region.is_some() {
            if let Some(e164) = e164(number, region) {
                found.phone(number, number, Some(e164));
            }
        } else {
            found.phone(number, number, None);
        }
    }
}

/// Whether the span `span`, which `after` follows, is written as a value
/// other than a telephone number: what libphonenumber's matcher leaves out
/// (a date or a time stamp, whatever its separators, and a span an `:mm`
/// follows), a range of years, decimals, or an ISBN or an EAN.
fn written_as_another_value(span: &str, after: &str) -> bool {
    followed_by_minutes(after)
        || is_date(span)
        || is_year_range(span)
        || is_decimals(span)
        || is_product_code(span)
}

/// Whether `after` begins with the minutes of a time, `:00` to `:59`, so
/// that the span before it ends on an hour.
fn followed_by_minutes(after: &str) -> bool {
    let mut characters = after.chars();
    characters.next() == Some(':')
        && characters
            .next()
            .is_some_and(|tens| ('0'..='5').contains(&tens))
        && characters.next().is_some_and(|ones| ones.is_ascii_digit())
}

/// Whether `span` is two years joined by a hyphen, the first before the
/// second (`2005-2026`).
fn is_year_range(span: &str) -> bool {
    let year = |part: &str| {
        (part.len() == 4 && part.chars().all(|character| character.is_ascii_digit()))
            .then(|| part.parse::<u16>().ok())
            .flatten()
            .filter(|year| (1800..=2199).contains(year))
    };
    span.split_once('-')
        .and_then(|(first, second)| Some((year(first)?, year(second)?)))
        .is_some_and(|(first, second)| first < second)
}

/// Whether a word of `span`, its parts between spaces, is a decimal
/// number: digits, one dot, digits (`48.8566 2.3522`). A number grouped by
/// dots has more than one.
fn is_decimals(span: &str) -> bool {
    span.split(|character: char| character.is_whitespace())
        .any(|word| {
            word.split_once('.').is_some_and(|(whole, fraction)| {
                !whole.is_empty()
                    && !fraction.is_empty()
                    && whole.chars().all(|character| character.is_ascii_digit())
                    && fraction.chars().all(|character| character.is_ascii_digit())
            })
        })
}

/// Whether `span` is written as an ISBN or an EAN whose check digit holds:
/// 13 digits beginning with an ISBN's `978` or `979`, or 13 digits written
/// together (a number dialled from abroad, `00` first, aside); or an
/// ISBN-10 written in hyphenated groups, its check digit a group of its
/// own. A telephone number is rarely written either way, so a number whose
/// digits happen to check stays one.
fn is_product_code(span: &str) -> bool {
    let digits: Vec<u32> = span
        .chars()
        .filter_map(|character| character.to_digit(10))
        .collect();
    let together = span.chars().all(|character| character.is_ascii_digit());
    match digits.as_slice() {
        [first, second, third, ..] if digits.len() == 13 => {
            let isbn =
                [*first, *second, *third] == [9, 7, 8] || [*first, *second, *third] == [9, 7, 9];
            let abroad = [*first, *second] == [0, 0];
            (isbn || (together && !abroad)) && ean_checks(&digits)
        }
        _ if digits.len() == 10 => {
            span.contains('-')
                && span
                    .rsplit('-')
                    .next()
                    .is_some_and(|group| group.len() == 1)
                && isbn_10_checks(&digits)
        }
        _ => false,
    }
}

/// Whether the 13 digits `digits` end on their EAN-13 check digit.
fn ean_checks(digits: &[u32]) -> bool {
    let sum: u32 = digits
        .iter()
        .enumerate()
        .map(|(place, digit)| {
            if place.is_multiple_of(2) {
                *digit
            } else {
                digit * 3
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// Whether the 10 digits `digits` end on their ISBN-10 check digit.
fn isbn_10_checks(digits: &[u32]) -> bool {
    let sum: u32 = digits
        .iter()
        .zip((1..=10).rev())
        .map(|(digit, weight)| digit * weight)
        .sum();
    sum.is_multiple_of(11)
}

/// Whether `span` is a date written with digits alone: a year of four
/// digits, a month and a day, in either order, joined by one hyphen, dot
/// or space each (`2026-10-26`, `26.10.2026`, `26 10 2026`).
fn is_date(span: &str) -> bool {
    let Some(separator) = span.chars().find(|character| !character.is_ascii_digit()) else {
        return false;
    };
    let parts: Vec<&str> = span.split(separator).collect();
    let [first, second, third] = parts.as_slice() else {
        return false;
    };
    let number = |part: &str, length: std::ops::RangeInclusive<usize>| {
        (length.contains(&part.len()) && part.chars().all(|character| character.is_ascii_digit()))
            .then(|| part.parse::<u16>().ok())
            .flatten()
    };
    let month_and_day = |month: &str, day: &str| {
        number(month, 1..=2).is_some_and(|month| (1..=12).contains(&month))
            && number(day, 1..=2).is_some_and(|day| (1..=31).contains(&day))
    };
    matches!(separator, '-' | '.' | ' ' | '\u{a0}' | '\u{202f}')
        && ((number(first, 4..=4).is_some() && month_and_day(second, third))
            || (number(third, 4..=4).is_some()
                && (month_and_day(second, first) || month_and_day(first, second))))
}

/// `href` after `scheme`, ASCII case aside.
fn without_scheme<'a>(href: &'a str, scheme: &str) -> Option<&'a str> {
    href.get(..scheme.len())
        .filter(|start| start.eq_ignore_ascii_case(scheme))
        .map(|_| &href[scheme.len()..])
}

/// `value` with its percent-encoded bytes decoded; `value` itself when
/// they decode to no UTF-8.
fn percent_decoded(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = bytes
            .get(at + 1..at + 3)
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match (bytes[at], hex) {
            (b'%', Some(byte)) => {
                decoded.push(byte);
                at += 3;
            }
            (byte, _) => {
                decoded.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8(decoded).unwrap_or_else(|_| value.to_owned())
}

/// The region of a BCP 47 language tag: its two-letter region subtag,
/// after its language and any extended language or script; `None` when
/// it has none, or a numeric one, which names no country.
fn region(lang: Option<&str>) -> Option<Id> {
    let mut subtags = lang?.split(['-', '_']);
    let language = subtags.next()?;
    if language.len() < 2
        || !language
            .chars()
            .all(|character| character.is_ascii_alphabetic())
    {
        return None;
    }
    for subtag in subtags {
        let alphabetic = subtag
            .chars()
            .all(|character| character.is_ascii_alphabetic());
        match subtag.len() {
            3 | 4 if alphabetic => {}
            2 if alphabetic => return subtag.to_ascii_uppercase().parse().ok(),
            _ => return None,
        }
    }
    None
}

/// `number` in E.164 when it is a valid number: with its country code,
/// or national in `region`.
fn e164(number: &str, region: Option<Id>) -> Option<String> {
    let parsed: PhoneNumber = if number.starts_with('+') {
        phonenumber::parse(None, number).ok()?
    } else {
        phonenumber::parse(Some(region?), number).ok()?
    };
    phonenumber::is_valid(&parsed).then(|| parsed.format().mode(Mode::E164).to_string())
}

/// Whether `character` may stand in an address's local part.
fn is_local(character: char) -> bool {
    character.is_ascii_alphanumeric() || ".!#$%&'*+/=?^_`{|}~-".contains(character)
}

/// The length of the domain `text` begins with, by the HTML Standard's
/// valid e-mail address syntax: labels of letters, digits and hyphens,
/// each beginning and ending with a letter or digit, at most 63 long,
/// joined by dots; `None` when it begins with none or a label is too long.
fn domain_length(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut end = 0;
    loop {
        let label = &bytes[end..];
        if !label.first().is_some_and(u8::is_ascii_alphanumeric) {
            return end.checked_sub(1);
        }
        let run = label
            .iter()
            .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'-')
            .count();
        let length = label[..run]
            .iter()
            .rposition(u8::is_ascii_alphanumeric)
            .map_or(0, |last| last + 1);
        if length > 63 {
            return None;
        }
        end += length;
        if length < run || bytes.get(end) != Some(&b'.') {
            return Some(end);
        }
        // Past the dot, to the next label.
        end += 1;
    }
}

/// Whether `text` is, whole, a valid e-mail address.
fn is_email(text: &str) -> bool {
    text.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty()
            && local.chars().all(is_local)
            && domain_length(domain) == Some(domain.len())
    })
}

/// The valid e-mail addresses `text` writes, in order.
fn emails(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    // Where the last address ended: the next one's local part starts after.
    let mut after = 0;
    for (at, _) in text.match_indices('@') {
        if at < after {
            continue;
        }
        let start = text[after..at]
            .char_indices()
            .rev()
            .take_while(|&(_, character)| is_local(character))
            .last()
            .map_or(at, |(offset, _)| after + offset);
        if start == at {
            continue;
        }
        if let Some(length) = domain_length(&text[at + 1..]) {
            found.push(&text[start..=at + length]);
            after = at + 1 + length;
        }
    }
    found
}

/// Whether `character` may stand inside a telephone number written in
/// text, beside its digits.
fn is_number_separator(character: char) -> bool {
    matches!(
        character,
        ' ' | '\u{a0}' | '\u{202f}' | '-' | '.' | '(' | ')'
    )
}

/// The spans of `text` written as a telephone number may be: a `+`, a
/// digit or `(` after no letter or digit, then digits and separators,
/// ending on a digit, with 7 to 15 digits; each with the text after it.
fn numbers(text: &str) -> Vec<(&str, &str)> {
    let mut found = Vec::new();
    let mut characters = text.char_indices().peekable();
    let mut previous: Option<char> = None;
    while let Some((start, first)) = characters.next() {
        let begins = (first == '+' || first == '(' || first.is_ascii_digit())
            && !previous.is_some_and(char::is_alphanumeric);
        previous = Some(first);
        if !begins {
            continue;
        }
        let mut end = start + first.len_utf8();
        let mut last_digit = first.is_ascii_digit().then_some(end);
        while let Some(&(at, character)) = characters.peek() {
            if !(character.is_ascii_digit() || is_number_separator(character)) {
                break;
            }
            characters.next();
            previous = Some(character);
            end = at + character.len_utf8();
            if character.is_ascii_digit() {
                last_digit = Some(end);
            }
        }
        let Some(last_digit) = last_digit else {
            continue;
        };
        let span = &text[start..last_digit];
        let digits = span.chars().filter(char::is_ascii_digit).count();
        // A letter or digit right after the span makes it part of a word.
        let glued = text[end..]
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric)
            && end == last_digit;
        if DIGITS.contains(&digits) && !glued {
            found.push((span, &text[last_digit..]));
        }
    }
    found
}
