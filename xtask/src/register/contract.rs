//! ADR 0140's inventory: each Shared path and the counts of its Cost contract,
//! each count a number and the work it names, read from the contract's text.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;

use super::document;

pub(crate) struct SharedPath {
    /// `F1`, as the table sets it in bold.
    pub(crate) id: String,
    pub(crate) counts: Vec<Count>,
}

/// One count of a contract: `at most 1 HTML parse`.
pub(crate) struct Count {
    /// The work it names, as the contract writes it: `html parse`.
    pub(crate) label: String,
    /// The label's words, each reduced to its stem.
    pub(crate) stems: BTreeSet<String>,
    pub(crate) cap: u32,
}

/// The words that end a count's name: what follows says where or when it
/// applies. A clause that starts with `otherwise` or `work`, as in `0
/// otherwise`, restates a count beside it and names no work of its own.
const ENDS: [&str; 19] = [
    "against",
    "and",
    "at",
    "by",
    "from",
    "however",
    "in",
    "into",
    "of",
    "on",
    "or",
    "otherwise",
    "per",
    "straight",
    "through",
    "to",
    "when",
    "with",
    "work",
];

/// The words a count's name can hold that say nothing of the work.
const FILLERS: [&str; 3] = ["its", "more", "s"];

/// The inventory in ADR 0140's `text`; `None` when its table is missing.
pub(crate) fn read(text: &str) -> Option<Vec<SharedPath>> {
    static ID: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\*\*(\w+)\*\*").expect("invariant: a valid pattern"));
    let rows = document::table(
        text,
        "### The Shared paths",
        &["Path", "Cost contract, per unit"],
    )?;
    rows.iter()
        .map(|row| {
            Some(SharedPath {
                id: ID.captures(row.cell("Path"))?[1].to_owned(),
                counts: counts(row.cell("Cost contract, per unit")),
            })
        })
        .collect()
}

/// Each count of a contract. What parentheses hold is an aside, never a count.
fn counts(contract: &str) -> Vec<Count> {
    static ASIDE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\([^()]*\)").expect("invariant: a valid pattern"));
    static COUNT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b(\d+)×? +([^,;:\d]+)").expect("invariant: a valid pattern")
    });
    let contract = ASIDE.replace_all(contract, "");
    COUNT
        .captures_iter(&contract)
        .filter_map(|count| {
            let words: Vec<String> = words(&count[2])
                .into_iter()
                .take_while(|word| !ENDS.contains(&word.as_str()))
                .filter(|word| !FILLERS.contains(&word.as_str()))
                .collect();
            (!words.is_empty()).then(|| Count {
                label: words.join(" "),
                stems: words.iter().map(|word| stem(word)).collect(),
                cap: count[1].parse().unwrap_or(u32::MAX),
            })
        })
        .collect()
}

/// The stems of every word of a row's Metric.
pub(crate) fn stems(text: &str) -> BTreeSet<String> {
    words(text).iter().map(|word| stem(word)).collect()
}

/// The lower-case words of `text`; a code span such as `/proc/meminfo` or
/// `Page.createIsolatedWorld` is one word.
fn words(text: &str) -> Vec<String> {
    text.split(|letter: char| !(letter.is_alphanumeric() || "_./".contains(letter)))
        .map(|word| word.trim_end_matches('.').to_lowercase())
        .filter(|word| !word.is_empty())
        .collect()
}

/// `word` without its plural or verb ending, so that `file opens` meets
/// `files opened` and `entry` meets `entries`.
fn stem(word: &str) -> String {
    if let Some(root) = word.strip_suffix("ies") {
        return format!("{root}y");
    }
    if ["ches", "shes", "sses", "xes"]
        .iter()
        .any(|ending| word.ends_with(ending))
    {
        return word[..word.len() - 2].to_owned();
    }
    if word.ends_with("ss") {
        return word.to_owned();
    }
    let root = word
        .strip_suffix('s')
        .or_else(|| word.strip_suffix("ed").filter(|_| word.len() > 4))
        .or_else(|| word.strip_suffix("ing").filter(|_| word.len() > 5));
    root.unwrap_or(word).to_owned()
}
