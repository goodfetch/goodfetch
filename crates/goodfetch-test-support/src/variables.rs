//! What test support reads from the process that runs a test (ADRs 0059 to
//! 0062): the running test's name, the seed and its `GOODFETCH_TEST_SEED`
//! override, the suite `just test` names in `GOODFETCH_TEST_SUITE`, the tool
//! manifest's path, the folder Journey variants write their result documents
//! into, the exercise records' folder with the test's ID as nextest gives
//! it, and on Windows the variables a child needs to start.
#![expect(
    clippy::disallowed_methods,
    reason = "test support reads the test's own variables, never a product's environment (ADR 0062)"
)]

use std::collections::BTreeMap;

/// Overrides every test's seed (ADR 0060).
pub const SEED_VARIABLE: &str = "GOODFETCH_TEST_SEED";

/// The running suite, which `just test <suite>` sets (ADR 0061).
pub const SUITE_VARIABLE: &str = "GOODFETCH_TEST_SUITE";

/// The path of the test tool manifest, which `just test` writes.
pub const TOOLS_VARIABLE: &str = "GOODFETCH_TEST_TOOLS";

/// The folder each Journey variant run writes its result document into,
/// which `just test` sets; unset, a run writes it in the test's own folder.
pub const RESULT_DOCUMENTS_VARIABLE: &str = "GOODFETCH_TEST_RESULT_DOCUMENTS";

/// The folder of every test's exercise record, which `just test` sets
/// (ADR 0072).
pub const RECORDS_VARIABLE: &str = "GOODFETCH_TEST_RECORDS";

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
            RESULT_DOCUMENTS_VARIABLE,
            RECORDS_VARIABLE,
            BINARY_ID_VARIABLE,
            TEST_NAME_VARIABLE,
            COVERAGE_VARIABLE,
        ]
        .into_iter()
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
            Some(other) => Err(SuiteError(other.to_owned())),
        }
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
#[error("{SUITE_VARIABLE} is `{0}`, not quick, pre-merge or release")]
pub struct SuiteError(pub String);

#[cfg(test)]
mod tests;
