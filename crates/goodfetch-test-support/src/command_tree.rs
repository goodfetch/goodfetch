//! What a Product's executable's tests read of its built `clap` command
//! line: each command's path, the help it renders, and its flags. Collie's
//! and Foxhound's crates each test their own executable, and neither can
//! reach the other's commands.

/// Every command path below `command`, each group before its commands,
/// hidden ones included.
#[must_use]
pub fn command_paths(command: &clap::Command) -> Vec<Vec<String>> {
    command
        .get_subcommands()
        .flat_map(|sub| {
            std::iter::once(vec![sub.get_name().to_owned()]).chain(
                command_paths(sub).into_iter().map(|mut path| {
                    path.insert(0, sub.get_name().to_owned());
                    path
                }),
            )
        })
        .collect()
}

/// The help the executable prints for `path`, rendered by its own command
/// line.
///
/// # Panics
///
/// When `--help` at `path` does not end the parse, as it does for a path of
/// [`command_paths`].
#[must_use]
pub fn command_help(command: &clap::Command, path: &[String]) -> String {
    let arguments = std::iter::once(command.get_name().to_owned())
        .chain(path.iter().cloned())
        .chain(std::iter::once("--help".to_owned()));
    command
        .clone()
        .try_get_matches_from(arguments)
        .expect_err("`--help` ends the parse")
        .to_string()
}

/// Every long flag of `command` and of each command below it, its visible
/// aliases among them.
#[must_use]
pub fn long_flags(command: &clap::Command) -> Vec<String> {
    command
        .get_arguments()
        .flat_map(|argument| {
            argument
                .get_long_and_visible_aliases()
                .into_iter()
                .flatten()
                .map(str::to_owned)
        })
        .chain(command.get_subcommands().flat_map(long_flags))
        .collect()
}

/// Each option that a command of `command`'s executable, named in
/// `markdown` on a `console` block's `$ ` line or in a code span, gives
/// although its command takes none of that name, as `<path> --<option>`:
/// what a skill names must exist (story 11-237).
#[must_use]
pub fn unknown_options(command: &clap::Command, markdown: &str) -> Vec<String> {
    let mut built = command.clone();
    built.build();
    let mut unknown = Vec::new();
    let mut fence: Option<&str> = None;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if let Some(info) = trimmed.strip_prefix("```") {
            fence = match fence {
                Some(_) => None,
                None => Some(info.trim()),
            };
            continue;
        }
        let named: Vec<&str> = match fence {
            Some("console") => trimmed.strip_prefix("$ ").into_iter().collect(),
            Some(_) => Vec::new(),
            None => line.split('`').skip(1).step_by(2).collect(),
        };
        for words in named {
            unknown.extend(options_lacked(&built, words));
        }
    }
    unknown
}

/// The options the command line `words` gives that its command lacks, when
/// it starts with `executable`'s name.
fn options_lacked(executable: &clap::Command, words: &str) -> Vec<String> {
    let mut words = words.split_whitespace();
    if words.next() != Some(executable.get_name()) {
        return Vec::new();
    }
    let mut command = executable;
    let mut path: Vec<&str> = Vec::new();
    let mut lacked = Vec::new();
    for word in words {
        if let Some(option) = word.strip_prefix("--").filter(|option| !option.is_empty()) {
            let name = option.split_once('=').map_or(option, |(name, _)| name);
            let known = command
                .get_arguments()
                .any(|argument| argument.get_long() == Some(name));
            if !known {
                lacked.push(format!("{} --{name}", path.join(" ")));
            }
        } else if let Some(sub) = command.find_subcommand(word) {
            command = sub;
            path.push(word);
        }
    }
    lacked
}

/// Those of `flags` whose name holds a word naming certificates or turning
/// their checks off; not `verif` alone, which `--require-verified` (a
/// Verdict's) holds.
#[must_use]
pub fn certificate_verification_switches(flags: &[String]) -> Vec<String> {
    const WORDS: [&str; 7] = [
        "insecure",
        "no-verify",
        "skip-verif",
        "cert",
        "tls",
        "ssl",
        "danger",
    ];
    flags
        .iter()
        .filter(|flag| {
            let flag = flag.to_ascii_lowercase();
            WORDS.iter().any(|word| flag.contains(word))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests;
