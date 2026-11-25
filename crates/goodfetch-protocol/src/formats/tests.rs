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

/// The shared stores' rule, case by case (ADR 0098): a store opens in any
/// minor of its format's major, and records the lowest minor of its Product
/// major that can use it, which the engine's own Product minor must meet.
#[rstest]
#[case::same_version(
    version(1, 4),
    version(1, 4),
    version(3, 2),
    2,
    Compatibility::Compatible
)]
#[case::older_minor(
    version(1, 4),
    version(1, 2),
    version(3, 2),
    0,
    Compatibility::Compatible
)]
#[case::newer_minor_whose_lowest_minor_the_engine_meets(
    version(1, 4),
    version(1, 6),
    version(3, 2),
    2,
    Compatibility::Compatible
)]
#[case::newer_minor_whose_lowest_minor_is_newer(
    version(1, 4),
    version(1, 6),
    version(3, 2),
    3,
    Compatibility::NewerMinor
)]
#[case::its_own_minor_whose_lowest_minor_is_newer(
    version(1, 4),
    version(1, 4),
    version(3, 2),
    5,
    Compatibility::NewerMinor
)]
#[case::older_major(
    version(2, 0),
    version(1, 9),
    version(3, 2),
    0,
    Compatibility::OlderMajor
)]
#[case::newer_major(
    version(1, 9),
    version(2, 0),
    version(3, 2),
    0,
    Compatibility::NewerMajor
)]
#[case::a_preview_engine_meets_its_lines_own_minor(
    version(1, 4),
    version(1, 4),
    version(0, 4),
    0,
    Compatibility::Compatible
)]
#[case::a_preview_engine_refuses_a_raised_store(
    version(1, 4),
    version(1, 4),
    version(0, 4),
    1,
    Compatibility::NewerMinor
)]
#[case::a_newer_preview_format(
    version(0, 1),
    version(0, 2),
    version(3, 2),
    0,
    Compatibility::NewerMajor
)]
fn the_shared_store_rule_opens_a_store_whose_lowest_minor_the_engine_meets(
    #[case] reader: ProtocolVersion,
    #[case] found: ProtocolVersion,
    #[case] engine: ProtocolVersion,
    #[case] lowest_minor: u16,
    #[case] compatibility: Compatibility,
) {
    assert_eq!(
        Rule::SharedStore.compatibility_of_store(reader, found, engine, lowest_minor),
        compatibility
    );
}

/// During `0.y` previews each `0.y` minor counts as a major (ADR 0030), for
/// every rule but the Usage ledger family's, whose major never changes.
#[rstest]
#[case::default_same(Rule::Default, version(0, 4), version(0, 4), Compatibility::Compatible)]
#[case::default_older(Rule::Default, version(0, 4), version(0, 3), Compatibility::OlderMajor)]
#[case::default_newer(Rule::Default, version(0, 4), version(0, 5), Compatibility::NewerMajor)]
#[case::default_first_major(Rule::Default, version(0, 9), version(1, 0), Compatibility::NewerMajor)]
#[case::default_last_preview(
    Rule::Default,
    version(1, 0),
    version(0, 9),
    Compatibility::OlderMajor
)]
#[case::shared_store_older(
    Rule::SharedStore,
    version(0, 4),
    version(0, 3),
    Compatibility::OlderMajor
)]
#[case::shared_store_newer(
    Rule::SharedStore,
    version(0, 4),
    version(0, 5),
    Compatibility::NewerMajor
)]
#[case::lower_minor_older(
    Rule::LowerMinor,
    version(0, 4),
    version(0, 3),
    Compatibility::OlderMajor
)]
#[case::lower_minor_newer(
    Rule::LowerMinor,
    version(0, 4),
    version(0, 5),
    Compatibility::NewerMajor
)]
#[case::frozen_major_newer(
    Rule::FrozenMajor,
    version(0, 4),
    version(0, 5),
    Compatibility::Compatible
)]
#[case::frozen_major_older(
    Rule::FrozenMajor,
    version(0, 4),
    version(0, 3),
    Compatibility::Compatible
)]
fn each_preview_minor_counts_as_a_major(
    #[case] rule: Rule,
    #[case] reader: ProtocolVersion,
    #[case] found: ProtocolVersion,
    #[case] compatibility: Compatibility,
) {
    assert_eq!(rule.compatibility(reader, found), compatibility);
}

/// The Clean-up rules' newest-readable rule, case by case (ADRs 0030 and
/// 0047): a reader reads a version's manifest of its own major up to its
/// own minor, as the default rule does, and skips any other one for the
/// newest version it can read.
/// Covers: format:clean-up-rules
#[rstest]
#[case::same_version(version(1, 4), version(1, 4), Compatibility::Compatible)]
#[case::older_minor(version(1, 4), version(1, 0), Compatibility::Compatible)]
#[case::newer_minor(version(1, 4), version(1, 5), Compatibility::NewerMinor)]
#[case::older_major(version(2, 0), version(1, 9), Compatibility::OlderMajor)]
#[case::newer_major(version(1, 9), version(2, 0), Compatibility::NewerMajor)]
fn the_newest_readable_rule_reads_a_manifest_of_its_major_up_to_its_own_minor(
    #[case] reader: ProtocolVersion,
    #[case] found: ProtocolVersion,
    #[case] compatibility: Compatibility,
) {
    assert_eq!(
        Rule::NewestReadable.compatibility(reader, found),
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
            ("clean-up-rules", Rule::NewestReadable),
            ("run-index", Rule::SharedStore),
            ("run-state", Rule::Default),
            ("installed-executables", Rule::FrozenMajor),
            ("usage-ledger", Rule::FrozenMajor),
            ("politeness-file", Rule::FrozenMajor),
            ("audit-log", Rule::FrozenMajor),
            ("known-certificates", Rule::FrozenMajor),
            ("operator-policy", Rule::SharedStore),
            ("user-configuration", Rule::SharedStore),
            ("export", Rule::Default),
            ("recipe-manifest", Rule::Default)
        ]
    );
}
