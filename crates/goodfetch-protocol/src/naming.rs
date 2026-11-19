//! How each language spells a catalog name's `snake_case` words (ADR 0099):
//! initialisms from its closed list are capitals in TypeScript names and
//! Python class names, and lowercase where they open a camelCase name, and
//! one word in Rust; a Python keyword takes a trailing underscore, and a Rust
//! keyword is a raw identifier. The one naming rule, which `xtask codegen`
//! writes the SDKs with and the engine renders its messages' names in.

use schemars::SchemaGenerator;

use crate::catalog::Event;
use crate::connection::Form;

/// ADR 0099's closed list of initialisms.
const INITIALISMS: [&str; 16] = [
    "ai", "api", "cdp", "css", "dom", "html", "http", "https", "js", "json", "mcp", "ok", "pdf",
    "uri", "url", "xml",
];

const PYTHON_KEYWORDS: [&str; 35] = [
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

/// Rust 2024's strict and reserved keywords that a raw identifier escapes.
const RUST_KEYWORDS: [&str; 48] = [
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "do", "dyn",
    "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl", "in", "let",
    "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref", "return",
    "static", "struct", "trait", "true", "try", "type", "typeof", "unsafe", "unsized", "use",
    "virtual", "where", "while", "yield",
];

/// The keywords Rust cannot make raw identifiers.
const RUST_UNRAW: [&str; 3] = ["crate", "self", "super"];

/// The words of a catalog name: its dotted parts' `snake_case` words.
fn words(name: &str) -> impl Iterator<Item = &str> {
    name.split(['.', '_']).filter(|word| !word.is_empty())
}

fn capitalized(word: &str) -> String {
    if INITIALISMS.contains(&word) {
        return word.to_uppercase();
    }
    let mut characters = word.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
}

/// A class or type name: `page_check_failed` gives `PageCheckFailed`.
pub fn pascal(name: &str) -> String {
    words(name).map(capitalized).collect()
}

/// A TypeScript member name: `page.wait_for_url` gives `pageWaitForURL`.
pub fn camel(name: &str) -> String {
    let mut words = words(name);
    let first = words.next().unwrap_or_default().to_owned();
    words.fold(first, |mut text, word| {
        text.push_str(&capitalized(word));
        text
    })
}

/// A Python member name: `page.wait_for_url` gives `page_wait_for_url`, and
/// a keyword gets a trailing underscore.
pub fn snake(name: &str) -> String {
    let text = words(name).collect::<Vec<_>>().join("_");
    if PYTHON_KEYWORDS.contains(&text.as_str()) {
        format!("{text}_")
    } else {
        text
    }
}

/// A Rust member name: `page.wait_for_url` gives `page_wait_for_url`, and a
/// keyword is a raw identifier, `r#continue`. `None` when Rust cannot make
/// the name raw (`self`, `super`, `crate`).
pub fn rust_snake(name: &str) -> Option<String> {
    let text = words(name).collect::<Vec<_>>().join("_");
    if RUST_UNRAW.contains(&text.as_str()) {
        None
    } else if RUST_KEYWORDS.contains(&text.as_str()) {
        Some(format!("r#{text}"))
    } else {
        Some(text)
    }
}

/// A Rust type name, an initialism being one word: `cdp_session` gives
/// `CdpSession`. `None` when the name is `Self`, which Rust cannot make raw.
pub fn rust_type(name: &str) -> Option<String> {
    let text: String = words(name)
        .map(|word| {
            let mut characters = word.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(characters).collect()
            })
        })
        .collect();
    (text != "Self").then_some(text)
}

/// An operation's name as `form`'s user writes the call, each dotted part
/// by its language's rule: `page.wait_for_url` gives `page.waitForURL` in
/// TypeScript and stays `page.wait_for_url` in Python and Rust. The CLI, the
/// tools and the catalog keep the catalog's name, their renderings being
/// their own designs (ADR 0079); so does a part Rust cannot spell.
pub fn operation(form: Form, name: &str) -> String {
    let spell = |part: &str| match form {
        Form::Typescript => camel(part),
        Form::Python => snake(part),
        Form::Rust => rust_snake(part).unwrap_or_else(|| part.to_owned()),
        Form::Cli | Form::Tool | Form::Catalog => part.to_owned(),
    };
    name.split('.').map(spell).collect::<Vec<_>>().join(".")
}

/// A setting key (ADR 0083) as `form`'s user sets it: `crawl.payload.max_size`
/// gives `crawl.payload.maxSize` in TypeScript, stays so in Python, whose
/// segments are dict keys and escape no keyword, and gives the module path
/// of its constant, `crawl::payload::MAX_SIZE`, in Rust. The CLI, the tools
/// and the catalog keep the key (ADR 0099).
pub fn setting(form: Form, key: &str) -> String {
    match form {
        Form::Typescript => key.split('.').map(camel).collect::<Vec<_>>().join("."),
        Form::Python => key
            .split('.')
            .map(|segment| words(segment).collect::<Vec<_>>().join("_"))
            .collect::<Vec<_>>()
            .join("."),
        Form::Rust => {
            let (modules, constant) = key.rsplit_once('.').unwrap_or(("", key));
            let constant = words(constant).collect::<Vec<_>>().join("_").to_uppercase();
            modules
                .split('.')
                .filter(|module| !module.is_empty())
                .map(|module| rust_snake(module).unwrap_or_else(|| module.to_owned()))
                .chain([constant])
                .collect::<Vec<_>>()
                .join("::")
        }
        Form::Cli | Form::Tool | Form::Catalog => key.to_owned(),
    }
}

/// A parameter as `form`'s user passes it: an option `maxRedirects` in
/// TypeScript, a keyword argument `max_redirects` in Python, and a builder
/// method `max_redirects` in Rust, each escaped as its language escapes a
/// keyword. The CLI, the tools and the catalog keep the catalog's name.
pub fn parameter(form: Form, name: &str) -> String {
    match form {
        Form::Typescript => camel(name),
        Form::Python => snake(name),
        Form::Rust => rust_snake(name).unwrap_or_else(|| name.to_owned()),
        Form::Cli | Form::Tool | Form::Catalog => name.to_owned(),
    }
}

/// The Rust marker type of the event `name`, as a listener's `on` takes it
/// (ADR 0081), whose payload is the type `payload`: the event's name in
/// upper camel case, or its payload's where that would end in `Error`, the
/// suffix of ADR 0079's error types, so `page_error` is `WebError`. `None`
/// when the name is `Self`.
pub fn event_marker(name: &str, payload: &str) -> Option<String> {
    let marker = rust_type(name)?;
    Some(if marker.ends_with("Error") {
        payload.to_owned()
    } else {
        marker
    })
}

/// The catalog's `event` as a listener's `on` takes it, its namespace left
/// out, with the name of its payload's type: `page.page_error` gives
/// `page_error` and `WebError`. The name is `None` for a payload the
/// schema bundle inlines, which no event has.
pub fn listened(event: &Event) -> (&'static str, Option<String>) {
    let name = event
        .name
        .rsplit_once('.')
        .map_or(event.name, |(_, name)| name);
    let payload = (event.payload)(&mut SchemaGenerator::default());
    let payload = payload
        .get("$ref")
        .and_then(serde_json::Value::as_str)
        .and_then(|reference| reference.rsplit('/').next())
        .map(str::to_owned);
    (name, payload)
}

/// The event `name`, the value a listener's `on` takes, whose payload is the
/// type `payload`, as `form`'s user listens to it (ADR 0099): its words
/// joined in lowercase in TypeScript and Python, as Playwright and the DOM
/// spell events (`requestfinished`), and its marker type in Rust
/// (`events::RequestFinished`). The CLI, the tools and the catalog keep the
/// wire's name.
pub fn event(form: Form, name: &str, payload: &str) -> String {
    match form {
        Form::Typescript | Form::Python => words(name).collect(),
        Form::Rust => event_marker(name, payload)
            .map_or_else(|| name.to_owned(), |marker| format!("events::{marker}")),
        Form::Cli | Form::Tool | Form::Catalog => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;

    use super::{
        Form, INITIALISMS, camel, event, listened, operation, parameter, pascal, rust_snake,
        rust_type, setting, snake,
    };
    use crate::page::events::{PAGE_PAGE_ERROR, PAGE_WEB_SOCKET};

    /// A word of a catalog name: an initialism, or two to eight characters
    /// that start with a letter. A single letter or a leading digit after
    /// the first word is outside the rule's round trip (`x_a_i` and `x_ai`
    /// both spell `xAI`), which is why codegen refuses two entries that give
    /// one receiver one member.
    fn word() -> impl Strategy<Value = String> {
        let letter = || proptest::sample::select(('a'..='z').collect::<Vec<_>>());
        let later = proptest::sample::select(('a'..='z').chain('0'..='9').collect::<Vec<_>>());
        let ordinary = (letter(), proptest::collection::vec(later, 1..8))
            .prop_map(|(first, rest)| std::iter::once(first).chain(rest).collect::<String>());
        prop_oneof![
            proptest::sample::select(INITIALISMS.to_vec()).prop_map(str::to_owned),
            ordinary,
        ]
    }

    /// A catalog name's words, one to five of them.
    fn name_words() -> impl Strategy<Value = Vec<String>> {
        proptest::collection::vec(word(), 1..6)
    }

    /// The words a camelCase spelling was made of, read back by the rule
    /// alone: a capital before a lowercase letter or a digit opens an
    /// ordinary word, and any other capital opens the longest initialism
    /// that ends where the next word, or nothing, begins.
    fn words_of(spelled: &str) -> Vec<String> {
        let characters: Vec<char> = spelled.chars().collect();
        let opens_a_word =
            |index: usize| characters.get(index).is_none_or(char::is_ascii_uppercase);
        let mut found = Vec::new();
        let mut index = 0;
        while index < characters.len() {
            let initialism = INITIALISMS
                .iter()
                .filter(|initialism| {
                    let end = index + initialism.len();
                    index > 0
                        && opens_a_word(index + 1)
                        && opens_a_word(end)
                        && characters.get(index..end).is_some_and(|slice| {
                            slice.iter().collect::<String>() == initialism.to_uppercase()
                        })
                })
                .max_by_key(|initialism| initialism.len());
            if let Some(initialism) = initialism {
                found.push((*initialism).to_owned());
                index += initialism.len();
                continue;
            }
            let mut word = String::from(characters[index].to_ascii_lowercase());
            index += 1;
            while !opens_a_word(index) {
                word.push(characters[index]);
                index += 1;
            }
            found.push(word);
        }
        found
    }

    /// The words an `UpperCamelCase` Rust type was made of: each capital opens
    /// one, an initialism included.
    fn rust_words_of(spelled: &str) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        for character in spelled.chars() {
            match found.last_mut() {
                Some(word) if !character.is_ascii_uppercase() => word.push(character),
                _ => found.push(String::from(character.to_ascii_lowercase())),
            }
        }
        found
    }

    /// The words a `snake_case` member was made of, once its escape is gone.
    fn snake_words_of(spelled: &str) -> Vec<String> {
        spelled
            .trim_start_matches("r#")
            .trim_end_matches('_')
            .split('_')
            .map(str::to_owned)
            .collect()
    }

    proptest! {
        #![proptest_config(goodfetch_test_support::proptest_config())]

        /// Covers: story:11-176
        #[test]
        fn a_camel_case_name_reads_back_as_its_words(words in name_words()) {
            let spelled = camel(&words.join("_"));

            prop_assert_eq!(words_of(&spelled), words);
        }

        /// Python's members and Rust's members and types read back as the
        /// words they were made of, each language by its own rule.
        ///
        /// Covers: story:11-176
        #[test]
        fn a_python_or_rust_name_reads_back_as_its_words(words in name_words()) {
            let name = words.join("_");

            prop_assert_eq!(snake_words_of(&snake(&name)), words.clone());
            if let Some(member) = rust_snake(&name) {
                prop_assert_eq!(snake_words_of(&member), words.clone());
            }
            if let Some(spelled) = rust_type(&name) {
                prop_assert_eq!(rust_words_of(&spelled), words);
            }
        }

        /// Covers: story:11-176
        #[test]
        fn two_names_never_share_a_spelling(first in name_words(), second in name_words()) {
            prop_assume!(first != second);
            let (first, second) = (first.join("_"), second.join("_"));

            prop_assert_ne!(camel(&first), camel(&second));
            prop_assert_ne!(pascal(&first), pascal(&second));
            prop_assert_ne!(snake(&first), snake(&second));
            if let (Some(first), Some(second)) = (rust_snake(&first), rust_snake(&second)) {
                prop_assert_ne!(first, second);
            }
            if let (Some(first), Some(second)) = (rust_type(&first), rust_type(&second)) {
                prop_assert_ne!(first, second);
            }
        }

        /// A name spells the same whichever of `.` and `_` parts its words:
        /// the spelling is stable when an entry moves to another namespace
        /// of the same words.
        ///
        /// Covers: story:11-176
        #[test]
        fn a_spelling_is_the_same_whatever_parts_its_words(name in name_words()) {
            let (dotted, joined) = (name.join("."), name.join("_"));

            prop_assert_eq!(camel(&dotted), camel(&joined));
            prop_assert_eq!(pascal(&dotted), pascal(&joined));
            prop_assert_eq!(snake(&dotted), snake(&joined));
            prop_assert_eq!(rust_snake(&dotted), rust_snake(&joined));
            prop_assert_eq!(rust_type(&dotted), rust_type(&joined));
        }
    }

    #[rstest]
    #[case::words("page_check_failed", "PageCheckFailed")]
    #[case::initialism("cdp_session", "CDPSession")]
    #[case::dotted("sample.echo", "SampleEcho")]
    fn pascal_capitalizes_each_word(#[case] name: &str, #[case] expected: &str) {
        assert_eq!(pascal(name), expected);
    }

    #[rstest]
    #[case::operation("sample.echo", "sampleEcho")]
    #[case::initialism_inside("page.wait_for_url", "pageWaitForURL")]
    #[case::initialism_first("url", "url")]
    fn camel_lowercases_only_the_first_word(#[case] name: &str, #[case] expected: &str) {
        assert_eq!(camel(name), expected);
    }

    #[rstest]
    #[case::operation("sample.echo", "sample_echo")]
    #[case::keyword("continue", "continue_")]
    fn snake_joins_words_and_escapes_keywords(#[case] name: &str, #[case] expected: &str) {
        assert_eq!(snake(name), expected);
    }

    /// One entry in every language: the case ADR 0099's table gives.
    ///
    /// Covers: story:11-176
    #[rstest]
    #[case::initialism_member(
        "page.wait_for_url",
        "pageWaitForURL",
        "page_wait_for_url",
        "page_wait_for_url"
    )]
    #[case::keyword("continue", "continue", "continue_", "r#continue")]
    #[case::reserved_keyword("gen", "gen", "gen", "r#gen")]
    fn a_member_spells_by_each_languages_rule(
        #[case] name: &str,
        #[case] typescript: &str,
        #[case] python: &str,
        #[case] rust: &str,
    ) {
        assert_eq!(
            (camel(name), snake(name), rust_snake(name)),
            (
                typescript.to_owned(),
                python.to_owned(),
                Some(rust.to_owned())
            )
        );
    }

    /// Covers: story:11-176
    #[rstest]
    #[case::initialism("cdp_session", "CDPSession", "CdpSession")]
    #[case::initialism_first("json_value", "JSONValue", "JsonValue")]
    #[case::dotted("sample.echo", "SampleEcho", "SampleEcho")]
    fn a_type_spells_by_each_languages_rule(
        #[case] name: &str,
        #[case] typescript_and_python: &str,
        #[case] rust: &str,
    ) {
        assert_eq!(
            (pascal(name), rust_type(name)),
            (typescript_and_python.to_owned(), Some(rust.to_owned()))
        );
    }

    /// Covers: story:11-176
    #[rstest]
    #[case::self_member("self")]
    #[case::super_member("super")]
    #[case::crate_member("crate")]
    fn rust_cannot_spell_a_member_it_cannot_make_raw(#[case] name: &str) {
        assert_eq!(rust_snake(name), None);
    }

    /// Covers: story:11-176
    #[test]
    fn rust_cannot_spell_the_self_type() {
        assert_eq!(rust_type("self"), None);
    }

    /// An operation reads as its caller writes the call, each dotted part
    /// by the form's rule; a form with no language of its own keeps the
    /// catalog's name.
    #[rstest]
    #[case::typescript(Form::Typescript, "page.wait_for_url", "page.waitForURL")]
    #[case::python(Form::Python, "page.wait_for_url", "page.wait_for_url")]
    #[case::rust(Form::Rust, "page.wait_for_url", "page.wait_for_url")]
    #[case::cli(Form::Cli, "page.wait_for_url", "page.wait_for_url")]
    #[case::tool(Form::Tool, "page.wait_for_url", "page.wait_for_url")]
    #[case::catalog(Form::Catalog, "page.wait_for_url", "page.wait_for_url")]
    #[case::typescript_receiver(
        Form::Typescript,
        "key_value_store.get_value",
        "keyValueStore.getValue"
    )]
    #[case::python_keyword(Form::Python, "route.continue", "route.continue_")]
    #[case::rust_keyword(Form::Rust, "route.continue", "route.r#continue")]
    #[case::rust_unraw(Form::Rust, "page.self", "page.self")]
    fn an_operation_spells_by_its_callers_form(
        #[case] form: Form,
        #[case] name: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(operation(form, name), expected);
    }

    /// A setting key reads as its caller sets it (ADR 0099's casing table):
    /// nested camelCase segments, nested snake_case segments, or Rust's
    /// modules and an UPPER_SNAKE constant; a form with no language of its
    /// own keeps ADR 0083's key.
    #[rstest]
    #[case::typescript(Form::Typescript, "crawl.payload.max_size", "crawl.payload.maxSize")]
    #[case::python(Form::Python, "crawl.payload.max_size", "crawl.payload.max_size")]
    #[case::rust(Form::Rust, "crawl.payload.max_size", "crawl::payload::MAX_SIZE")]
    #[case::cli(Form::Cli, "crawl.payload.max_size", "crawl.payload.max_size")]
    #[case::tool(Form::Tool, "crawl.payload.max_size", "crawl.payload.max_size")]
    #[case::catalog(Form::Catalog, "crawl.payload.max_size", "crawl.payload.max_size")]
    #[case::typescript_initialism(Form::Typescript, "http.max_url_length", "http.maxURLLength")]
    #[case::rust_module_words(Form::Rust, "clean_up.fetch_timeout", "clean_up::FETCH_TIMEOUT")]
    #[case::rust_raw_module(Form::Rust, "type.size", "r#type::SIZE")]
    fn a_setting_key_spells_by_its_callers_form(
        #[case] form: Form,
        #[case] key: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(setting(form, key), expected);
    }

    /// A parameter reads as its caller passes it: an option's camelCase
    /// in TypeScript, a keyword argument in Python, a builder method in
    /// Rust, each escaped as the language escapes a keyword.
    #[rstest]
    #[case::typescript(Form::Typescript, "max_redirects", "maxRedirects")]
    #[case::python(Form::Python, "max_redirects", "max_redirects")]
    #[case::rust(Form::Rust, "max_redirects", "max_redirects")]
    #[case::cli(Form::Cli, "max_redirects", "max_redirects")]
    #[case::tool(Form::Tool, "max_redirects", "max_redirects")]
    #[case::catalog(Form::Catalog, "max_redirects", "max_redirects")]
    #[case::typescript_initialism(Form::Typescript, "base_url", "baseURL")]
    #[case::python_keyword(Form::Python, "from", "from_")]
    #[case::rust_keyword(Form::Rust, "type", "r#type")]
    fn a_parameter_spells_by_its_callers_form(
        #[case] form: Form,
        #[case] name: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(parameter(form, name), expected);
    }

    /// An event reads as its caller listens to it (ADR 0099): its words
    /// joined in lowercase, as Playwright and the DOM spell events, in
    /// TypeScript and Python, and Rust's marker type, which takes its
    /// payload's name where its own would end in `Error`.
    #[rstest]
    #[case::typescript(Form::Typescript, "request_finished", "PageRequest", "requestfinished")]
    #[case::python(Form::Python, "request_finished", "PageRequest", "requestfinished")]
    #[case::rust(
        Form::Rust,
        "request_finished",
        "PageRequest",
        "events::RequestFinished"
    )]
    #[case::cli(Form::Cli, "request_finished", "PageRequest", "request_finished")]
    #[case::tool(Form::Tool, "request_finished", "PageRequest", "request_finished")]
    #[case::catalog(Form::Catalog, "request_finished", "PageRequest", "request_finished")]
    #[case::typescript_error(Form::Typescript, "page_error", "WebError", "pageerror")]
    #[case::rust_error(Form::Rust, "page_error", "WebError", "events::WebError")]
    fn an_event_spells_by_its_callers_form(
        #[case] form: Form,
        #[case] name: &str,
        #[case] payload: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(event(form, name, payload), expected);
    }

    /// A catalog event reads as a listener names it, with its payload's
    /// type, which Rust's marker of `page_error` takes.
    #[test]
    fn a_catalog_event_reads_as_its_listener_names_it() {
        assert_eq!(
            listened(&PAGE_PAGE_ERROR),
            ("page_error", Some("WebError".to_owned()))
        );
        assert_eq!(listened(&PAGE_WEB_SOCKET).0, "web_socket");
    }
}
