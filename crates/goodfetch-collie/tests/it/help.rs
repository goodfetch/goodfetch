//! `--help` through Collie's executable is the help its own command line
//! renders in process, generated commands and hand-written ones alike, in
//! English (ADRs 0069 and 0073): the help a person reads is the command
//! tree the binary runs.

use goodfetch_collie::executable::command;
use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::{TestEnv, cli};
use rstest::rstest;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The help Collie's command line renders for `path`, as its executable
/// builds it, with its version.
fn rendered(path: &[&str]) -> String {
    let arguments = std::iter::once("goodfetch-collie")
        .chain(path.iter().copied())
        .chain(std::iter::once("--help"));
    command(Product::Collie)
        .version(env!("CARGO_PKG_VERSION"))
        .try_get_matches_from(arguments)
        .unwrap_err()
        .to_string()
}

/// Covers: story:11-219
#[rstest]
#[case::run_start(&["run", "start"])]
#[case::run_finish(&["run", "finish"])]
#[case::engine_info(&["engine", "info"])]
#[case::sample_echo(&["sample", "echo"])]
#[case::config_explain(&["config", "explain"])]
#[case::browser_clean(&["browser", "clean"])]
fn the_executable_prints_the_help_its_command_line_renders(#[case] path: &[&str]) {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE).args(path).arg("--help").run().unwrap();

    assert_eq!(
        (output.exit, output.stdout.as_str()),
        (Some(0), rendered(path).as_str())
    );
}
