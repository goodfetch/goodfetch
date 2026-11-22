//! What test support reads from the process that runs a test (ADRs 0059 to
//! 0062): the running test's name, the seed and its `GOODFETCH_TEST_SEED`
//! override, the suite `just test` names in `GOODFETCH_TEST_SUITE`, the tool
//! manifest's path, the folder Journey variants write their result documents
//! into, the exercise records' folder with the test's ID as nextest gives
//! it, the way a runner declares `DnsNamespace` comes up on it, the cap on
//! the Callgrind children a counts benchmark runs at once, what a
//! Cargo a test starts needs to find its toolchain, and on Windows the
//! variables a child needs to start.
#![expect(
    clippy::disallowed_methods,
    reason = "test support reads the test's own variables, never a product's environment (ADR 0062)"
)]

use std::collections::BTreeMap;
use std::num::NonZeroUsize;

/// Overrides every test's seed (ADR 0060).
pub const SEED_VARIABLE: &str = "GOODFETCH_TEST_SEED";

/// The running suite, which `just test <suite>` sets (ADR 0061).
pub const SUITE_VARIABLE: &str = "GOODFETCH_TEST_SUITE";

/// The path of the test tool manifest, which `just test` writes.
pub const TOOLS_VARIABLE: &str = "GOODFETCH_TEST_TOOLS";

/// The path of the pinned LazyFS build, which `just lazyfs-test` sets
/// (ADR 0060).
pub const LAZYFS_VARIABLE: &str = "GOODFETCH_LAZYFS";

/// The folder each LazyFS test's folder is made in, in place of `/tmp`
/// (`TestEnv::for_lazyfs`): one where the operating system lets the user
/// mount a FUSE file system, with a short path for the home's Unix sockets.
pub const LAZYFS_FOLDER_VARIABLE: &str = "GOODFETCH_LAZYFS_FOLDER";

/// The executable a skew run of `just journeys --engine <path>` drives
/// (ADR 0066): when it is set, every Journey variant runs it in place of the
/// executable its test passes.
pub const ENGINE_VARIABLE: &str = "GOODFETCH_TEST_ENGINE";

/// The folder each Journey variant run also writes its result document
/// into, beside the test's own folder, which `just test` sets.
pub const RESULT_DOCUMENTS_VARIABLE: &str = "GOODFETCH_TEST_RESULT_DOCUMENTS";

/// The folder of the case logs, where a Rust test that runs a Client
/// conformance case appends its entry (ADR 0069), which `just test` sets.
pub const CASE_LOGS_VARIABLE: &str = "GOODFETCH_TEST_CASE_LOGS";

/// The folder of every test's exercise record, which `just test` sets
/// (ADR 0072).
pub const RECORDS_VARIABLE: &str = "GOODFETCH_TEST_RECORDS";

/// The way a runner declares `DnsNamespace` comes up on it, `user_namespace`
/// or `sudo`, so a silent fallback to the other fails (ADR 0067); unset, it
/// tries the first, then the second.
pub const DNS_NAMESPACE_VARIABLE: &str = "GOODFETCH_TEST_DNS_NAMESPACE";

/// The most Callgrind children a counts benchmark runs at once, below its
/// own bound of half the cores, at most eight: `xtask counts
/// --callgrind-at-once` sets it, or the host's environment does (#2520).
pub const CALLGRIND_AT_ONCE_VARIABLE: &str = "GOODFETCH_COUNTS_CALLGRIND_AT_ONCE";

/// Set where the tests run with no GUI session, as the Release suite's
/// macOS cells do on a runner started in the Background session (#3224):
/// Chrome for Testing's full build cannot start headless there, so each
/// test's user configuration sets `browser.headless_shell`, its Browser
/// cache holds the shell, and a test that needs the full build skips.
pub const NO_GUI_SESSION_VARIABLE: &str = "GOODFETCH_TEST_NO_GUI_SESSION";

/// Where Playwright keeps its browsers when it is not its default folder
/// (the CI image sets it), from which the tests' Browser cache takes the
/// Chrome for Testing build Playwright pins.
pub(crate) const PLAYWRIGHT_BROWSERS_VARIABLE: &str = "PLAYWRIGHT_BROWSERS_PATH";

/// The user's folders, under which Playwright's default folder is: `HOME`
/// on macOS and Linux, `LOCALAPPDATA` on Windows.
pub(crate) const USER_FOLDER_VARIABLES: [&str; 2] = ["HOME", "LOCALAPPDATA"];

/// What a Cargo a test starts needs to find its toolchain, since the test's
/// map clears the environment and gives `HOME` a folder of its own: the
/// `PATH` of the running test, Cargo's and rustup's homes, and the
/// toolchain rustup picked for the test's own build.
pub(crate) const TOOLCHAIN_VARIABLES: [&str; 4] =
    ["PATH", "CARGO_HOME", "RUSTUP_HOME", "RUSTUP_TOOLCHAIN"];

/// The user's own folder on Windows, under which Cargo's and rustup's homes
/// are when no variable names them; `HOME` on macOS and Linux.
pub(crate) const WINDOWS_USER_VARIABLE: &str = "USERPROFILE";

/// The binary ID of the test nextest runs (`<package>::<target>`).
pub(crate) const BINARY_ID_VARIABLE: &str = "NEXTEST_BINARY_ID";

/// The name of the test nextest runs, within its binary.
pub(crate) const TEST_NAME_VARIABLE: &str = "NEXTEST_TEST_NAME";

/// Where an executable built for coverage writes its profile: cargo-llvm-cov
/// sets it under `just coverage` (ADR 0072), and each executable a test
/// starts gets it.
pub(crate) const COVERAGE_VARIABLE: &str = "LLVM_PROFILE_FILE";

/// What Windows needs in a child's environment to start a process.
pub(crate) const WINDOWS_VARIABLES: [&str; 3] = ["SystemRoot", "ComSpec", "PATHEXT"];

/// The largest seed: every language reads a JSON integer up to 2^53 - 1
/// exactly, so a seed in a description is the same number in each.
pub const MAX_SEED: u64 = (1 << 53) - 1;

/// The variables test support reads, taken once from the process or given
/// by a test of test support itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Variables {
    values: BTreeMap<String, String>,
}

impl Variables {
    /// The variables of the running process that test support reads; one
    /// that is not Unicode counts as absent.
    pub fn from_process() -> Self {
        [
            SEED_VARIABLE,
            SUITE_VARIABLE,
            TOOLS_VARIABLE,
            LAZYFS_VARIABLE,
            LAZYFS_FOLDER_VARIABLE,
            RESULT_DOCUMENTS_VARIABLE,
            CASE_LOGS_VARIABLE,
            ENGINE_VARIABLE,
            RECORDS_VARIABLE,
            BINARY_ID_VARIABLE,
            TEST_NAME_VARIABLE,
            COVERAGE_VARIABLE,
            DNS_NAMESPACE_VARIABLE,
            CALLGRIND_AT_ONCE_VARIABLE,
            PLAYWRIGHT_BROWSERS_VARIABLE,
            NO_GUI_SESSION_VARIABLE,
        ]
        .into_iter()
        .chain(USER_FOLDER_VARIABLES)
        .chain(TOOLCHAIN_VARIABLES)
        .chain([WINDOWS_USER_VARIABLE])
        .chain(WINDOWS_VARIABLES)
        .filter_map(|name| std::env::var(name).ok().map(|value| (name, value)))
        .collect()
    }

    /// One variable's value.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    /// The seed of the test with this full name: a hash of the name, so two
    /// tests never share a stream, unless `GOODFETCH_TEST_SEED` overrides it.
    pub fn seed(&self, test: &str) -> Result<u64, SeedError> {
        let Some(value) = self.get(SEED_VARIABLE) else {
            return Ok(name_hash(test) >> 11);
        };
        let seed: u64 = value.parse().map_err(|_| SeedError::NotANumber)?;
        if seed > MAX_SEED {
            return Err(SeedError::TooLarge);
        }
        Ok(seed)
    }

    /// The running suite: `GOODFETCH_TEST_SUITE`, or the Quick suite when it
    /// is absent, so a test run by hand never takes more cases than Quick.
    pub fn suite(&self) -> Result<Suite, SuiteError> {
        match self.get(SUITE_VARIABLE) {
            None | Some("quick") => Ok(Suite::Quick),
            Some("pre-merge") => Ok(Suite::PreMerge),
            Some("release") => Ok(Suite::Release),
            Some(other) => Err(SuiteError {
                value: other.to_owned(),
            }),
        }
    }

    /// Whether the tests run with no GUI session:
    /// `GOODFETCH_TEST_NO_GUI_SESSION` is set, to any value.
    pub fn no_gui_session(&self) -> bool {
        self.get(NO_GUI_SESSION_VARIABLE).is_some()
    }

    /// The cap on the Callgrind children a counts benchmark runs at once:
    /// `GOODFETCH_COUNTS_CALLGRIND_AT_ONCE`, or none when it is absent.
    pub fn callgrind_at_once(&self) -> Result<Option<NonZeroUsize>, CallgrindAtOnceError> {
        self.get(CALLGRIND_AT_ONCE_VARIABLE)
            .map(|value| {
                value.parse().map_err(|_| CallgrindAtOnceError {
                    value: value.to_owned(),
                })
            })
            .transpose()
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for Variables {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(pairs: I) -> Self {
        Self {
            values: pairs
                .into_iter()
                .map(|(name, value)| (name.into(), value.into()))
                .collect(),
        }
    }
}

/// 64-bit FNV-1a: fixed by its definition, unlike the standard library's
/// hasher, so a test's seed is the same on every Rust version.
fn name_hash(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The full name of the running test: libtest names each test's thread
/// after it (`module::test`).
pub fn current_test() -> String {
    std::thread::current()
        .name()
        .unwrap_or("unnamed test")
        .to_owned()
}

/// The folder of the case logs, which `just test` names in
/// `GOODFETCH_TEST_CASE_LOGS`: a Rust test passes it to
/// `ClientCaseRun::start`, which keeps no entry when it is absent (ADR 0069).
pub fn case_logs() -> Option<std::path::PathBuf> {
    Variables::from_process()
        .get(CASE_LOGS_VARIABLE)
        .map(std::path::PathBuf::from)
}

/// One of the three suites (ADR 0061).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suite {
    /// The Quick suite, also every run by hand.
    Quick,
    /// The Pre-merge suite.
    PreMerge,
    /// The Release suite.
    Release,
}

impl Suite {
    /// Its name as `just test` and `GOODFETCH_TEST_SUITE` write it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::PreMerge => "pre-merge",
            Self::Release => "release",
        }
    }
}

/// Why `GOODFETCH_TEST_SEED` cannot be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SeedError {
    /// It is no unsigned integer.
    #[error("{SEED_VARIABLE} is not an unsigned integer")]
    NotANumber,
    /// It is above 2^53 - 1, which not every language reads exactly.
    #[error("{SEED_VARIABLE} is above {MAX_SEED}, the largest seed every language reads exactly")]
    TooLarge,
}

/// `GOODFETCH_TEST_SUITE` names no suite.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{SUITE_VARIABLE} is `{value}`, not quick, pre-merge or release")]
#[non_exhaustive]
pub struct SuiteError {
    /// The value it holds.
    pub value: String,
}

/// `GOODFETCH_COUNTS_CALLGRIND_AT_ONCE` is no whole number of 1 or more.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{CALLGRIND_AT_ONCE_VARIABLE} is `{value}`, not a whole number of 1 or more")]
#[non_exhaustive]
pub struct CallgrindAtOnceError {
    /// The value it holds.
    pub value: String,
}

#[cfg(test)]
mod tests;
