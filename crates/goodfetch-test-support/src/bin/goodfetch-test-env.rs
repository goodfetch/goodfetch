//! `goodfetch-test-env` (ADR 0062): a test's setup for TypeScript and
//! Python. It builds a `TestEnv`, prints its description as one JSON line on
//! stdout, and tears it down when its stdin closes. It exits non-zero, with a
//! message on stderr, when a start fails. `--print-schemas` prints the JSON
//! Schemas of the crate's file types for `xtask codegen` and starts nothing.

use std::io::{self, Read as _, Write as _};
use std::process::ExitCode;

use clap::Parser;
use goodfetch_test_support::{Description, TestEnv, Variables, schemas, tools};

#[derive(Parser)]
#[command(
    name = "goodfetch-test-env",
    about = "Prepares one test's setup and prints its description"
)]
struct Arguments {
    /// The locale tag whose `LANG` the environment takes, such as `tr-TR`.
    #[arg(long)]
    locale: Option<String>,
    /// The time zone the environment's `TZ` names, such as `Asia/Kathmandu`.
    #[arg(long)]
    time_zone: Option<String>,
    /// The calling test's full name, from which its seed and folder name
    /// come.
    #[arg(long, default_value = "goodfetch-test-env")]
    test_name: String,
    /// Prints the JSON Schemas of test support's file types, keyed by their
    /// paths under `test-support/fixtures/`, and starts nothing.
    #[arg(long, exclusive = true)]
    print_schemas: bool,
}

fn main() -> ExitCode {
    let arguments = Arguments::parse();
    let result = if arguments.print_schemas {
        print_schemas()
    } else {
        serve(&arguments)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            #[expect(
                clippy::print_stderr,
                reason = "goodfetch-test-env reports a failed start on its own stderr (ADR 0062)"
            )]
            {
                eprintln!("goodfetch-test-env: {error}");
            }
            ExitCode::FAILURE
        }
    }
}

type Failure = Box<dyn std::error::Error>;

fn print_schemas() -> Result<(), Failure> {
    let schemas = serde_json::to_string(&schemas())?;
    write_line(&schemas)
}

fn serve(arguments: &Arguments) -> Result<(), Failure> {
    let mut env = TestEnv::create(&arguments.test_name, &Variables::from_process())?;
    if let Some(tag) = &arguments.locale {
        env = env.with_locale(tag);
    }
    if let Some(zone) = &arguments.time_zone {
        env = env.with_time_zone(zone);
    }
    let description = Description::new(&env, tools()?);
    write_line(&serde_json::to_string(&description)?)?;
    #[expect(
        clippy::disallowed_methods,
        reason = "goodfetch-test-env tears down when its stdin closes (ADR 0062)"
    )]
    io::stdin().read_to_end(&mut Vec::new())?;
    drop(env);
    Ok(())
}

fn write_line(line: &str) -> Result<(), Failure> {
    #[expect(
        clippy::disallowed_methods,
        reason = "goodfetch-test-env writes its description to its own stdout (ADR 0062)"
    )]
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{line}")?;
    stdout.flush()?;
    Ok(())
}
