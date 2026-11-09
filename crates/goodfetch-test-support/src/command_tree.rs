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
