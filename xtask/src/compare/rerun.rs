//! The command that reruns a comparison run: the recipe and every argument
//! the run was given beyond the recipe's own.

use std::path::Path;

use anyhow::{Result, bail};
use clap::ValueEnum as _;

use super::{
    COLLIE_ARMS_DRIVER, COLLIE_DRIVER, CRAWLEE, FOXHOUND_PROGRAM, RunArguments, SCRAPY,
    TRIALS_FILE, trials,
};

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
    for setting in &arguments.foxhound_settings {
        give("--foxhound-setting", Some(setting.clone()));
    }
    for cell in &arguments.cells {
        give("--cell", Some(cell.clone()));
    }
    if let Some(browser) = &arguments.browser {
        give("--browser", Some(browser.display().to_string()));
    }
    for (flag, path, default) in [
        ("--crawlee", &arguments.crawlee, CRAWLEE),
        ("--scrapy", &arguments.scrapy, SCRAPY),
        (
            "--foxhound-program",
            &arguments.foxhound_program,
            FOXHOUND_PROGRAM,
        ),
        ("--collie-driver", &arguments.collie_driver, COLLIE_DRIVER),
        (
            "--collie-arms-driver",
            &arguments.collie_arms_driver,
            COLLIE_ARMS_DRIVER,
        ),
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

/// The words of `command`, a command [`of`] wrote, each read back as a
/// POSIX shell reads it: split at spaces, a single-quoted span taken as it
/// is and a backslash outside one taking the next character as it is, so
/// `'\''` gives a quote.
pub(super) fn words(command: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quoted = false;
    let mut characters = command.chars();
    while let Some(character) = characters.next() {
        match (quoted, character) {
            (false, ' ') => words.extend(word.take()),
            (false, '\'') => {
                word.get_or_insert_default();
                quoted = true;
            }
            (false, '\\') => {
                let Some(escaped) = characters.next() else {
                    bail!("the rerun command `{command}` ends in a backslash");
                };
                word.get_or_insert_default().push(escaped);
            }
            (true, '\'') => quoted = false,
            (_, character) => word.get_or_insert_default().push(character),
        }
    }
    if quoted {
        bail!("the rerun command `{command}` leaves a quote open");
    }
    words.extend(word);
    Ok(words)
}

#[cfg(test)]
mod tests;
