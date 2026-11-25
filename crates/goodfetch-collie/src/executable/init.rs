//! `init` (spec #608, issue 1422; ADRs 0087, 0096 and 0097; story:11-275,
//! story:11-278): writes a starter the executable embeds, a project in one
//! of the SDKs' languages that runs and tests itself, into a folder, the
//! working folder unless one is named, and prints each file it wrote and
//! each it left.
//!
//! Each executable embeds only its Product's starters: Collie's refuses
//! Foxhound's, naming Foxhound's executable, as a usage error, `2`. A
//! starter or language left out is asked on a terminal and is a usage error
//! anywhere else. A path already taken is never overwritten: it is left as
//! it is and listed in `skipped`. In each file, `{{name}}` becomes the
//! project's name, the folder's name in lowercase ASCII letters and digits
//! joined by `-`, and `{{version}}` the executable's Product version, so
//! that the project takes the SDK of the executable's own version (ADR
//! 0090).
#![expect(
    clippy::disallowed_methods,
    reason = "the command writes the project folder from the executable's own thread, before and outside any runtime"
)]

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::ValueEnum;
use goodfetch_engine::{CallError, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::checkpoint::{FILE_WRITE_FAILED, FileWriteFailedDetails, WriteSide};
use goodfetch_protocol::cli;
use goodfetch_protocol::starters::{FOXHOUND_STARTERS, InitResult, StarterLanguage};

use super::output::{self, ColourFlag, Human};

/// The project's name when its folder's name holds no ASCII letter or
/// digit.
const DEFAULT_NAME: &str = "goodfetch-starter";

/// The suffix of a starter file stored under another name than the one
/// `init` writes: Cargo leaves out of the crate's package any folder that
/// holds a `Cargo.toml`, so the Rust starter's is `Cargo.toml.template`.
const TEMPLATE: &str = ".template";

/// One file of a starter an executable embeds.
#[derive(Clone, Copy, Debug)]
pub struct StarterFile {
    /// Its `/`-separated path from the starters folder, the starter's name
    /// and its language first, such as `browser/typescript/package.json`.
    pub path: &'static str,
    /// Its bytes, with their placeholders.
    pub contents: &'static [u8],
}

/// A file of Collie's `starters/` folder, by its path from it.
macro_rules! starter_file {
    ($path:literal) => {
        StarterFile {
            path: $path,
            contents: include_bytes!(concat!("../../starters/", $path)),
        }
    };
}

/// The starters an executable embeds, which `init` writes.
#[derive(Clone, Copy, Debug)]
pub struct Starters {
    files: &'static [StarterFile],
}

impl Starters {
    /// Collie's executable's: its `starters/` folder, every file of which
    /// this list names.
    pub const COLLIE: Self = Self::new(&[
        starter_file!("browser/python/.gitignore"),
        starter_file!("browser/python/main.py"),
        starter_file!("browser/python/pyproject.toml"),
        starter_file!("browser/python/test_title.py"),
        starter_file!("browser/python/title.py"),
        starter_file!("browser/rust/.gitignore"),
        starter_file!("browser/rust/Cargo.toml.template"),
        starter_file!("browser/rust/src/main.rs"),
        starter_file!("browser/rust/src/tests.rs"),
        starter_file!("browser/typescript/.gitignore"),
        starter_file!("browser/typescript/package.json"),
        starter_file!("browser/typescript/src/main.ts"),
        starter_file!("browser/typescript/src/title.ts"),
        starter_file!("browser/typescript/test/title.test.ts"),
        starter_file!("browser/typescript/tsconfig.json"),
    ]);

    /// The starters of these files.
    pub const fn new(files: &'static [StarterFile]) -> Self {
        Self { files }
    }

    /// Their files.
    #[must_use]
    pub const fn files(self) -> &'static [StarterFile] {
        self.files
    }

    /// Each starter's name.
    fn names(self) -> BTreeSet<&'static str> {
        self.files
            .iter()
            .filter_map(|file| file.path.split_once('/').map(|(name, _)| name))
            .collect()
    }

    /// The files of `starter` in `language`, each by the path `init` writes
    /// it to from the project's folder.
    fn of(self, starter: &str, language: StarterLanguage) -> Vec<(&'static str, &'static [u8])> {
        self.files
            .iter()
            .filter_map(|file| {
                let (name, rest) = file.path.split_once('/')?;
                let (written_in, path) = rest.split_once('/')?;
                let path = path.strip_suffix(TEMPLATE).unwrap_or(path);
                (name == starter && written_in == language.as_str())
                    .then_some((path, file.contents))
            })
            .collect()
    }
}

/// Writes a starter project this executable carries, which runs and tests
/// itself, in TypeScript, Python or Rust, never replacing a file already
/// there.
#[derive(Clone, Debug, clap::Args)]
pub(super) struct Arguments {
    /// The project's folder, created when missing; the working folder
    /// when not given.
    directory: Option<PathBuf>,
    /// The starter to write, such as `browser`; asked on a terminal
    /// when not given.
    #[arg(long, value_name = "NAME")]
    starter: Option<String>,
    /// Its language; asked on a terminal when not given.
    #[arg(long = "lang", value_enum, value_name = "LANGUAGE")]
    language: Option<LanguageFlag>,
    /// Prints the result as JSON.
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    _colour: ColourFlag,
}

/// The languages `init --lang` names.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum LanguageFlag {
    Typescript,
    Python,
    Rust,
}

impl From<LanguageFlag> for StarterLanguage {
    fn from(flag: LanguageFlag) -> Self {
        match flag {
            LanguageFlag::Typescript => Self::Typescript,
            LanguageFlag::Python => Self::Python,
            LanguageFlag::Rust => Self::Rust,
        }
    }
}

/// What `init` is asked to write.
pub(crate) struct Request {
    /// The project's folder, created when missing.
    pub(crate) folder: PathBuf,
    /// The starter, when the command line names it.
    pub(crate) starter: Option<String>,
    /// Its language, when the command line names it.
    pub(crate) language: Option<StarterLanguage>,
    /// The executable's Product.
    pub(crate) product: Product,
    /// Its Product version.
    pub(crate) version: String,
}

/// Why `init` wrote nothing more.
#[derive(Debug)]
pub(crate) enum Refused {
    /// The command line, or the answer to a question, names no starter or
    /// language the executable writes; the message says why.
    Usage(String),
    /// This file or folder could not be written.
    Io { path: PathBuf, error: io::Error },
}

/// Writes the starter `request` names into its folder, asking `ask` for a
/// starter or language it leaves out: `ask` answers a person's answer, or
/// none when no person can be asked.
pub(crate) fn write(
    starters: Starters,
    request: Request,
    ask: &mut dyn FnMut(&str) -> Option<String>,
) -> Result<InitResult, Refused> {
    let names = starters.names();
    let offered = names.iter().copied().collect::<Vec<_>>().join(", ");
    let starter = match request.starter {
        Some(starter) => starter,
        None => ask(&format!("Which starter ({offered})?")).ok_or_else(|| {
            Refused::Usage(format!(
                "no starter given: name one with `--starter`, one of {offered}"
            ))
        })?,
    };
    if !names.contains(starter.as_str()) {
        return Err(Refused::Usage(unknown_starter(
            &starter,
            &offered,
            request.product,
        )));
    }
    let languages = StarterLanguage::ALL.map(StarterLanguage::as_str).join(", ");
    let language = if let Some(language) = request.language {
        language
    } else {
        let answer = ask(&format!("Which language ({languages})?")).ok_or_else(|| {
            Refused::Usage(format!(
                "no language given: name one with `--lang`, one of {languages}"
            ))
        })?;
        StarterLanguage::named(&answer).ok_or_else(|| {
            Refused::Usage(format!(
                "`{answer}` is no language `init` writes: one of {languages}"
            ))
        })?
    };
    let name = project_name(&request.folder);
    let mut written = Vec::new();
    let mut skipped = Vec::new();
    for (path, contents) in starters.of(&starter, language) {
        let target = request.folder.join(path);
        if write_new(&target, &filled(contents, &name, &request.version))? {
            written.push(path.to_owned());
        } else {
            skipped.push(path.to_owned());
        }
    }
    Ok(InitResult {
        starter,
        language,
        written,
        skipped,
    })
}

/// Why `starter` is no starter this executable writes: one only
/// Foxhound's executable embeds, named with the command that writes it
/// there, or none at all.
fn unknown_starter(starter: &str, offered: &str, product: Product) -> String {
    if product == Product::Collie && FOXHOUND_STARTERS.contains(&starter) {
        let foxhound = Product::Foxhound.executable_name();
        return format!(
            "`{starter}` is Foxhound's starter, which Collie's executable does not carry: run `{foxhound} init --starter {starter}`"
        );
    }
    format!("`{starter}` is no starter this executable carries: one of {offered}")
}

/// The project's name from its folder's: its ASCII letters and digits in
/// lowercase, each run of anything else one `-` between them.
fn project_name(folder: &Path) -> String {
    let folder = folder.file_name().unwrap_or_default().to_string_lossy();
    let name = folder
        .split(|letter: char| !letter.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .to_ascii_lowercase();
    if name.is_empty() {
        DEFAULT_NAME.to_owned()
    } else {
        name
    }
}

/// `contents` with the project's `name` and the executable's `version` in
/// place of their placeholders.
fn filled(contents: &[u8], name: &str, version: &str) -> Vec<u8> {
    String::from_utf8_lossy(contents)
        .replace("{{name}}", name)
        .replace("{{version}}", version)
        .into_bytes()
}

/// Writes `contents` to `target`, creating its folders, unless a file,
/// folder or link is already there: answers whether it wrote it.
fn write_new(target: &Path, contents: &[u8]) -> Result<bool, Refused> {
    let refused = |path: &Path| {
        let path = path.to_path_buf();
        move |error| Refused::Io { path, error }
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(refused(parent))?;
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target);
    match file {
        Ok(mut file) => {
            io::Write::write_all(&mut file, contents).map_err(refused(target))?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(refused(target)(error)),
    }
}

/// Runs `init` for the executable `identity` names, with the `starters` it
/// embeds, asking `human` for what the command line leaves out, and prints
/// the result.
pub(super) fn init(
    starters: Starters,
    Arguments {
        directory,
        starter,
        language,
        json,
        ..
    }: Arguments,
    identity: &Identity,
    human: &Human,
) -> ExitCode {
    let Ok(working) = std::env::current_dir() else {
        tracing::error!("the working folder cannot be read");
        return ExitCode::FAILURE;
    };
    let request = Request {
        folder: directory.map_or_else(|| working.clone(), |directory| working.join(directory)),
        starter,
        language: language.map(StarterLanguage::from),
        product: identity.product,
        version: identity.product_version.to_string(),
    };
    let result = match write(starters, request, &mut |question| human.ask(question)) {
        Ok(result) => result,
        Err(Refused::Usage(message)) => {
            human.failure(&message, &[]);
            return ExitCode::from(cli::USAGE.code);
        }
        Err(Refused::Io { path, error }) => {
            let failure = CallError::new(
                &FILE_WRITE_FAILED,
                format!(
                    "`{}` could not be written: {error}. The files written before it stay.",
                    path.display()
                ),
                &FileWriteFailedDetails {
                    reason: "other".to_owned(),
                    os_code: error.raw_os_error().map(|code| code.to_string()),
                    path: Some(path.display().to_string()),
                    side: WriteSide::Engine,
                },
            );
            return output::fail(failure.to_error_object(), None, json, human);
        }
    };
    if json {
        output::json(&result);
        return ExitCode::SUCCESS;
    }
    for path in &result.written {
        human.field("wrote", path);
    }
    for path in &result.skipped {
        human.field("skipped", &format!("{path}, already there"));
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests;
