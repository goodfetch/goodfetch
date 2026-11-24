//! The folders the `ci/*-fetch.sh` scripts download the tests' copies of
//! pinned builds into: each one's variable names the folder a test reads
//! its copy from, as `ci/windows.sh` sets them for the tests under Wine,
//! where no `HOME` gives the default folder.

use std::path::Path;

use goodfetch_test_support::{
    CHROME_FOR_TESTING_DOWNLOADS_VARIABLE, CLAUDE_CODE_DOWNLOADS_VARIABLE,
    LIGHTPANDA_DOWNLOADS_VARIABLE, TOXIPROXY_DOWNLOADS_VARIABLE, TestEnv,
};

/// Each downloads variable test support reads.
pub(crate) const VARIABLES: [&str; 4] = [
    TOXIPROXY_DOWNLOADS_VARIABLE,
    CLAUDE_CODE_DOWNLOADS_VARIABLE,
    LIGHTPANDA_DOWNLOADS_VARIABLE,
    CHROME_FOR_TESTING_DOWNLOADS_VARIABLE,
];

/// What the probe prints for a copy this platform has no build of.
pub(crate) const NO_BUILD: &str = "no build";

#[test]
fn each_downloads_variable_names_the_folder_its_copy_is_read_from() {
    let env = TestEnv::new();
    let mut command = crate::probe(&env, "probe::prints_where_each_copy_is_read_from");
    for variable in VARIABLES {
        command.env(variable, env.folder().join(variable));
    }

    let output = command.output().unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for variable in VARIABLES {
        let line = stdout
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{variable}=")))
            .unwrap_or_else(|| panic!("the probe printed no copy for {variable}: {stdout}"));
        assert!(
            line == NO_BUILD || Path::new(line).starts_with(env.folder().join(variable)),
            "{variable}'s copy is read from {line}"
        );
    }
}
