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

/// The Usage ledger family's rule, case by case (ADRs 0030 and 0098): its
/// major never changes, and newer minors only add, so a reader reads every
/// minor of its major and refuses another major.
#[rstest]
#[case::same_version(version(0, 1), version(0, 1), Compatibility::Compatible)]
#[case::older_minor(version(1, 4), version(1, 0), Compatibility::Compatible)]
#[case::newer_minor(version(1, 4), version(1, 9), Compatibility::Compatible)]
#[case::older_major(version(2, 0), version(1, 9), Compatibility::OlderMajor)]
#[case::newer_major(version(1, 9), version(2, 0), Compatibility::NewerMajor)]
fn the_frozen_major_rule_reads_every_minor_of_its_major(
    #[case] reader: ProtocolVersion,
    #[case] found: ProtocolVersion,
    #[case] compatibility: Compatibility,
) {
    assert_eq!(
        Rule::FrozenMajor.compatibility(reader, found),
        compatibility
    );
}

/// The shared stores' rule, case by case (ADR 0098): a store records the
/// lowest minor of its major that can use it, and a reader opens a newer
/// minor unless that lowest minor is newer than its own.
#[rstest]
#[case::same_version(version(1, 4), version(1, 4), 4, Compatibility::Compatible)]
#[case::older_minor(version(1, 4), version(1, 2), 1, Compatibility::Compatible)]
#[case::newer_minor_whose_lowest_minor_the_reader_meets(
    version(1, 4),
    version(1, 6),
    4,
    Compatibility::Compatible
)]
#[case::newer_minor_whose_lowest_minor_is_newer(
    version(1, 4),
    version(1, 6),
    5,
    Compatibility::NewerMinor
)]
#[case::older_major(version(2, 0), version(1, 9), 9, Compatibility::OlderMajor)]
#[case::newer_major(version(1, 9), version(2, 0), 0, Compatibility::NewerMajor)]
fn the_shared_store_rule_opens_a_newer_minor_whose_lowest_minor_the_reader_meets(
    #[case] reader: ProtocolVersion,
    #[case] found: ProtocolVersion,
    #[case] lowest_minor: u16,
    #[case] compatibility: Compatibility,
) {
    assert_eq!(
        Rule::SharedStore.compatibility_of_store(reader, found, lowest_minor),
        compatibility
    );
}

/// A shared store read without its lowest minor, as before the marker
/// exists, counts as needing its own minor (ADR 0098).
#[rstest]
#[case::older_minor(version(1, 4), version(1, 2), Compatibility::Compatible)]
#[case::newer_minor(version(1, 4), version(1, 5), Compatibility::NewerMinor)]
fn a_shared_store_read_without_its_lowest_minor_needs_its_own_minor(
    #[case] reader: ProtocolVersion,
    #[case] found: ProtocolVersion,
    #[case] compatibility: Compatibility,
) {
    assert_eq!(
        Rule::SharedStore.compatibility(reader, found),
        compatibility
    );
}

/// The rule of a handshake whose two sides settle on the lower minor, a
/// Browser host's with its engine (ADR 0096): only another major is refused.
#[rstest]
#[case::older_minor(version(1, 4), version(1, 0), Compatibility::Compatible)]
#[case::newer_minor(version(1, 4), version(1, 9), Compatibility::Compatible)]
#[case::older_major(version(2, 0), version(1, 9), Compatibility::OlderMajor)]
#[case::newer_major(version(1, 9), version(2, 0), Compatibility::NewerMajor)]
fn the_lower_minor_rule_refuses_only_another_major(
    #[case] reader: ProtocolVersion,
    #[case] found: ProtocolVersion,
    #[case] compatibility: Compatibility,
) {
    assert_eq!(Rule::LowerMinor.compatibility(reader, found), compatibility);
}

#[test]
fn the_registry_lists_every_format_under_its_rule() {
    let names: Vec<(&str, Rule)> = FORMATS
        .iter()
        .map(|format| (format.name, format.rule))
        .collect();

    assert_eq!(
        names,
        [
            ("protocol", Rule::Default),
            ("browser-cache", Rule::Default),
            ("run-index", Rule::SharedStore),
            ("run-state", Rule::Default),
            ("installed-executables", Rule::FrozenMajor),
            ("operator-policy", Rule::SharedStore),
            ("user-configuration", Rule::SharedStore)
        ]
    );
}
