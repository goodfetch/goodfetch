//! The command line Collie's executable prints for `xtask codegen` (ADR
//! 0072): the committed command lines hold Collie's tree as the executable
//! prints it, hidden commands included, and `goodfetch-cli`'s committed
//! namespaces file is the one its catalog gives. `xtask codegen --check`
//! reads both files without building the executable; this is where they
//! are checked against it.
//! Covers: command:command-line

use goodfetch_test_support::{TestEnv, cli};
use serde_json::Value;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Both executables' command lines, which `xtask codegen` commits.
const COMMAND_LINES: &str =
    include_str!("../../../goodfetch-protocol/generated/command_lines.json");

/// The namespaces of the generated commands, which `xtask codegen` commits.
const NAMESPACES: &str = include_str!("../../../goodfetch-cli/generated/namespaces.rs");

#[test]
fn the_committed_command_line_is_the_one_the_executable_prints() {
    let env = TestEnv::new();

    let output = cli(&env, COLLIE).arg("command-line").run().unwrap();

    assert_eq!(output.exit, Some(0), "{output}");
    let printed: Value = serde_json::from_str(&output.stdout).unwrap();
    let committed: Value = serde_json::from_str(COMMAND_LINES).unwrap();
    assert_eq!(
        (&printed["command_line"], printed["namespaces"].as_str()),
        (&committed["collie"], Some(NAMESPACES)),
        "run `just codegen` and commit what it writes"
    );
}
