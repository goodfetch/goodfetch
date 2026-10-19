//! A JUnit artifact's name, `junit-<suite>-<tier>-<os>-<arch>[-<runtime>]-<shard>`
//! (TESTING.md, Sharding, caching and reports), parsed into its parts. A
//! suite is one word, or `pre-merge`; the tier is what lies between it and
//! the OS, so it may hold hyphens (`pull-request`, `chromium-build`).

use anyhow::{Result, bail};

const PREFIX: &str = "junit-";

/// The OS a JUnit artifact names.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Os {
    Linux,
    Macos,
    Windows,
}

impl Os {
    const ALL: [Self; 3] = [Self::Linux, Self::Macos, Self::Windows];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
        }
    }
}

/// The architecture a JUnit artifact names.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Arch {
    X64,
    Arm64,
}

impl Arch {
    const ALL: [Self; 2] = [Self::X64, Self::Arm64];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::X64 => "x64",
            Self::Arm64 => "arm64",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Configuration {
    pub(crate) suite: String,
    pub(crate) tier: String,
    pub(crate) os: String,
    pub(crate) arch: String,
    pub(crate) runtime: Option<String>,
    pub(crate) shard: String,
}

/// Whether the artifact holds JUnit files.
pub(crate) fn is_junit(name: &str) -> bool {
    name.starts_with(PREFIX)
}

pub(crate) fn parse(name: &str) -> Result<Configuration> {
    let Some(rest) = name.strip_prefix(PREFIX) else {
        bail!("`{name}` is no JUnit artifact: its name starts with `{PREFIX}`");
    };
    let parts: Vec<&str> = rest.split('-').collect();
    let Some(os_at) = (1..parts.len().saturating_sub(2)).find(|&at| {
        Os::ALL.iter().any(|os| os.name() == parts[at])
            && Arch::ALL.iter().any(|arch| arch.name() == parts[at + 1])
    }) else {
        bail!("`{name}` is not `junit-<suite>-<tier>-<os>-<arch>[-<runtime>]-<shard>`");
    };
    let suite_words = if parts[..2] == ["pre", "merge"] { 2 } else { 1 };
    if os_at <= suite_words {
        bail!("`{name}` names no tier between its suite and its OS");
    }
    let shard = parts[parts.len() - 1];
    let runtime = &parts[os_at + 2..parts.len() - 1];
    Ok(Configuration {
        suite: parts[..suite_words].join("-"),
        tier: parts[suite_words..os_at].join("-"),
        os: parts[os_at].to_owned(),
        arch: parts[os_at + 1].to_owned(),
        runtime: (!runtime.is_empty()).then(|| runtime.join("-")),
        shard: shard.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{Configuration, parse};

    #[rstest]
    #[case::one_word_suite_and_tier(
        "junit-quick-nightly-linux-x64-1",
        ("quick", "nightly", "linux", "x64", None, "1")
    )]
    #[case::hyphenated_suite_and_tier(
        "junit-pre-merge-pull-request-linux-x64-1",
        ("pre-merge", "pull-request", "linux", "x64", None, "1")
    )]
    #[case::runtime(
        "junit-release-merge-queue-windows-arm64-node-24-3",
        ("release", "merge-queue", "windows", "arm64", Some("node-24"), "3")
    )]
    fn artifact_name_gives_its_configuration(
        #[case] name: &str,
        #[case] expected: (&str, &str, &str, &str, Option<&str>, &str),
    ) {
        let (suite, tier, os, arch, runtime, shard) = expected;

        let configuration = parse(name).unwrap();

        assert_eq!(
            configuration,
            Configuration {
                suite: suite.to_owned(),
                tier: tier.to_owned(),
                os: os.to_owned(),
                arch: arch.to_owned(),
                runtime: runtime.map(str::to_owned),
                shard: shard.to_owned(),
            }
        );
    }

    #[rstest]
    #[case::no_tier("junit-quick-linux-x64-1")]
    #[case::no_os("junit-quick-nightly-plan9-x64-1")]
    #[case::no_shard("junit-quick-nightly-linux-x64")]
    fn artifact_name_without_every_part_is_refused(#[case] name: &str) {
        assert!(parse(name).is_err());
    }
}
