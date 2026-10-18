//! How each language spells a catalog name's `snake_case` words (ADR 0099):
//! initialisms from its closed list are capitals in TypeScript names and
//! Python class names, and lowercase where they open a camelCase name; a
//! Python keyword takes a trailing underscore.

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
pub(super) fn pascal(name: &str) -> String {
    words(name).map(capitalized).collect()
}

/// A TypeScript member name: `page.wait_for_url` gives `pageWaitForURL`.
pub(super) fn camel(name: &str) -> String {
    let mut words = words(name);
    let first = words.next().unwrap_or_default().to_owned();
    words.fold(first, |mut text, word| {
        text.push_str(&capitalized(word));
        text
    })
}

/// A Python member name: `page.wait_for_url` gives `page_wait_for_url`, and
/// a keyword gets a trailing underscore.
pub(super) fn snake(name: &str) -> String {
    let text = words(name).collect::<Vec<_>>().join("_");
    if PYTHON_KEYWORDS.contains(&text.as_str()) {
        format!("{text}_")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{camel, pascal, snake};

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
}
