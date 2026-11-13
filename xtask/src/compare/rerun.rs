//! The command that reruns a comparison run: the recipe and every argument
//! the run was given beyond the recipe's own.

use std::path::Path;

use clap::ValueEnum as _;

use super::{CRAWLEE, RunArguments, SCRAPY, TRIALS_FILE, trials};

/// `word` as one word of a POSIX shell's command: as it is when no
/// character of it is special, otherwise in single quotes.
fn quoted(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_./=:,@%+".contains(character));
    if plain {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// The command that reruns a run started with `arguments`: `just compare`
/// with the profile, then each argument given beyond the recipe's own
/// (`--tools` and `--spider-builds`) whose value is not the default.
pub(super) fn of(arguments: &RunArguments) -> String {
    let mut words = vec![
        "just".to_owned(),
        "compare".to_owned(),
        arguments.spider_profile.clone(),
    ];
    let mut give = |flag: &str, value: Option<String>| {
        words.push(flag.to_owned());
        words.extend(value);
    };
    for scenario in &arguments.scenarios {
        if let Some(value) = scenario.to_possible_value() {
            give("--scenario", Some(value.get_name().to_owned()));
        }
    }
    if arguments.trials != trials::TRIALS {
        give("--trials", Some(arguments.trials.to_string()));
    }
    if arguments.growth {
        give("--growth", None);
    }
    if let Some(pages) = arguments.spider_pages {
        give("--spider-pages", Some(pages.to_string()));
    }
    if let Some(machine) = &arguments.machine {
        give("--machine", Some(machine.clone()));
    }
    if let Some(browser) = &arguments.browser {
        give("--browser", Some(browser.display().to_string()));
    }
    for (flag, path, default) in [
        ("--crawlee", &arguments.crawlee, CRAWLEE),
        ("--scrapy", &arguments.scrapy, SCRAPY),
        ("--out", &arguments.out, TRIALS_FILE),
    ] {
        if path != Path::new(default) {
            give(flag, Some(path.display().to_string()));
        }
    }
    words
        .iter()
        .map(|word| quoted(word))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests;
