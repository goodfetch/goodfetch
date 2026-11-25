use rstest::rstest;

use super::PluginProtocolRange;
use crate::catalog::ProtocolVersion;

const fn version(major: u16, minor: u16) -> ProtocolVersion {
    ProtocolVersion { major, minor }
}

const fn range(min: ProtocolVersion, max: ProtocolVersion) -> PluginProtocolRange {
    PluginProtocolRange { min, max }
}

/// Covers: story:7-77
#[rstest]
#[case::the_same(range(version(0, 1), version(0, 3)), Some(version(0, 3)))]
#[case::overlapping(range(version(0, 2), version(0, 5)), Some(version(0, 3)))]
#[case::within(range(version(0, 2), version(0, 2)), Some(version(0, 2)))]
#[case::touching(range(version(0, 0), version(0, 1)), Some(version(0, 1)))]
#[case::newer(range(version(0, 4), version(1, 0)), None)]
#[case::older(range(version(0, 0), version(0, 0)), None)]
fn two_ranges_share_their_newest_common_version(
    #[case] other: PluginProtocolRange,
    #[case] shared: Option<ProtocolVersion>,
) {
    let ours = range(version(0, 1), version(0, 3));

    assert_eq!(
        (ours.newest_shared(other), other.newest_shared(ours)),
        (shared, shared)
    );
}
