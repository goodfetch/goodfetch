//! `goodfetch-scripted-engine` (ADR 0069): the Scripted engine as a
//! process, for the faults neither Product's executable can produce on
//! demand (a refused `hello`, a crash mid-call, garbage on stdout). It
//! plays one script of `test-support/fixtures/engine-scripts/` over its own
//! stdio, records every message it saw, and exits non-zero, with the
//! failure's code and reason on stderr, on a message no step expects or a
//! step left unused.
//!
//! `--script <file> --mode stdio` names the script. An SDK reaches it as an
//! Attached engine instead: its Product's engine-path variable
//! (`GOODFETCH_COLLIE_ENGINE_PATH` or `GOODFETCH_FOXHOUND_ENGINE_PATH`,
//! ADR 0090) names this executable, which the SDK starts as
//! `attached-engine` with its environment map; the script is then
//! `scripted-engine/script.json` under the map's `GOODFETCH_HOME`, and the
//! record `scripted-engine/record.jsonl` beside it.
//!
//! A test-only executable the Simulation never runs, it opens its own
//! stdio and writes its own stdout, the one writer to stdout beyond
//! `STYLE.md`'s three (ADRs 0060 and 0069).

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use goodfetch_test_support::{ATTACHED_RECORD, ATTACHED_SCRIPT, Ending, Script, play};

#[derive(Parser)]
#[command(
    name = "goodfetch-scripted-engine",
    about = "Plays one script of protocol messages to a Client, as a stand-in engine in tests",
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true
)]
struct Arguments {
    /// The script to play.
    #[arg(long, required = true)]
    script: Option<PathBuf>,
    /// How the Client reaches the engine.
    #[arg(long, value_enum, required = true)]
    mode: Option<Mode>,
    /// Where to write the record of every message, one JSON line each.
    #[arg(long)]
    record: Option<PathBuf>,
    #[command(subcommand)]
    form: Option<Form>,
}

/// How the Client reaches the engine.
#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    /// Over the engine's own stdin and stdout, as an Attached engine.
    Stdio,
}

#[derive(Subcommand)]
enum Form {
    /// Runs as an SDK starts its Attached engine: plays
    /// `scripted-engine/script.json` under `GOODFETCH_HOME` over stdio.
    AttachedEngine,
}

fn main() -> ExitCode {
    let arguments = Arguments::parse();
    match run(&arguments) {
        Ok(Ending::Finished | Ending::Closed) => ExitCode::SUCCESS,
        Ok(Ending::Exit(code)) => ExitCode::from(code),
        Err(failure) => {
            #[expect(
                clippy::print_stderr,
                reason = "the Scripted engine reports a failed play on its own stderr (ADRs 0060 and 0069)"
            )]
            {
                eprintln!("goodfetch-scripted-engine: {failure}");
            }
            ExitCode::FAILURE
        }
    }
}

type Failure = Box<dyn std::error::Error>;

fn run(arguments: &Arguments) -> Result<Ending, Failure> {
    let (script, record) = match (&arguments.form, &arguments.script) {
        (Some(Form::AttachedEngine), _) => {
            let home = home()?;
            (home.join(ATTACHED_SCRIPT), Some(home.join(ATTACHED_RECORD)))
        }
        (None, Some(script)) => (script.clone(), arguments.record.clone()),
        (None, None) => return Err("invalid_arguments: --script names no script".into()),
    };
    let script = Script::read(&script).map_err(|error| format!("invalid_script: {error}"))?;
    let record: Box<dyn Write> = match record {
        Some(path) => Box::new(create(&path)?),
        None => Box::new(io::sink()),
    };
    #[expect(
        clippy::disallowed_methods,
        reason = "the test-only Scripted engine plays its script over its own stdio, outside `transport` (ADRs 0060 and 0069)"
    )]
    let (stdin, stdout) = (io::stdin().lock(), io::stdout().lock());
    Ok(play(&script, stdin, stdout, record)?)
}

/// The Goodfetch home the SDK's environment map names.
fn home() -> Result<PathBuf, Failure> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Scripted engine reads the Goodfetch home its Client's environment map gives it, as an engine's `main` does (ADR 0069)"
    )]
    let home = std::env::var_os("GOODFETCH_HOME");
    home.map(PathBuf::from).ok_or_else(|| {
        "no_home: GOODFETCH_HOME names no Goodfetch home to read the script from".into()
    })
}

fn create(path: &Path) -> Result<File, Failure> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Scripted engine is synchronous and starts no async worker to block (ADR 0069)"
    )]
    let file = File::create(path);
    file.map_err(|error| {
        format!(
            "record_failed: {} cannot be written: {error}",
            path.display()
        )
        .into()
    })
}
