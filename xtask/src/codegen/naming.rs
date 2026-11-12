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
    use proptest::prelude::*;
    use rstest::rstest;

    use super::{INITIALISMS, camel, pascal, snake};

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

    proptest! {
        #![proptest_config(goodfetch_test_support::proptest_config())]

        /// Covers: story:11-176
        #[test]
        fn a_camel_case_name_reads_back_as_its_words(words in name_words()) {
            let spelled = camel(&words.join("_"));

            prop_assert_eq!(words_of(&spelled), words);
        }

        /// Covers: story:11-176
        #[test]
        fn two_names_never_share_a_spelling(first in name_words(), second in name_words()) {
            prop_assume!(first != second);
            let (first, second) = (first.join("_"), second.join("_"));

            prop_assert_ne!(camel(&first), camel(&second));
            prop_assert_ne!(pascal(&first), pascal(&second));
        }

        /// A name spells the same whichever of `.` and `_` parts its words:
        /// the spelling is stable when an entry moves to another namespace
        /// of the same words.
        ///
        /// Covers: story:11-176
        #[test]
        fn a_spelling_is_the_same_whatever_parts_its_words(name in name_words()) {
            prop_assert_eq!(camel(&name.join(".")), camel(&name.join("_")));
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
}
