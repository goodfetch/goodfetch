use rstest::rstest;

use super::{Compatibility, FORMATS, Rule};
use crate::catalog::ProtocolVersion;

const fn version(major: u16, minor: u16) -> ProtocolVersion {
    ProtocolVersion { major, minor }
}

/// ADR 0030's default rule, case by case (ADR 0066): the reader is the
/// engine, or whatever opens the file; the found version is the Client's
/// `hello`, or the file's header.
#[rstest]
#[case::same_version(version(1, 4), version(1, 4), Compatibility::Compatible)]
#[case::older_minor(version(1, 4), version(1, 0), Compatibility::Compatible)]
#[case::newer_minor(version(1, 4), version(1, 5), Compatibility::NewerMinor)]
#[case::older_major(version(2, 0), version(1, 9), Compatibility::OlderMajor)]
#[case::newer_major(version(1, 9), version(2, 0), Compatibility::NewerMajor)]
fn the_default_rule_reads_its_major_up_to_its_own_minor(
    #[case] reader: ProtocolVersion,
    #[case] found: ProtocolVersion,
    #[case] compatibility: Compatibility,
) {
    assert_eq!(Rule::Default.compatibility(reader, found), compatibility);
}

#[test]
fn the_registry_lists_every_format_under_the_default_rule() {
    let names: Vec<(&str, Rule)> = FORMATS
        .iter()
        .map(|format| (format.name, format.rule))
        .collect();

    assert_eq!(
        names,
        [
            ("protocol", Rule::Default),
            ("browser-cache", Rule::Default),
            ("run-index", Rule::Default),
            ("run-state", Rule::Default),
            ("operator-policy", Rule::Default),
            ("user-configuration", Rule::Default)
        ]
    );
}
