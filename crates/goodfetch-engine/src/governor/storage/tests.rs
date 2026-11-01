//! The storage level through its interface (spec #213's Seam 4): a
//! filesystem's readings and the two settings in, its level out, with no
//! clock and no filesystem (ADR 0122).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::storage::StorageLevel;
use proptest::prelude::*;
use rstest::rstest;

use super::{FilesystemInodes, FilesystemReading, Thresholds, level};

const GIB: u64 = 1 << 30;

/// A filesystem of `size` with `free` left and no inode count.
fn space(size: u64, free: u64) -> FilesystemReading {
    FilesystemReading {
        size,
        free,
        inodes: None,
    }
}

/// A 100 GiB filesystem with plenty of space and `free` of 1,000 inodes.
fn inodes(free: u64) -> FilesystemReading {
    FilesystemReading {
        size: 100 * GIB,
        free: 50 * GIB,
        inodes: Some(FilesystemInodes { total: 1000, free }),
    }
}

const DEFAULTS: Thresholds = Thresholds {
    low_free: None,
    critical_free: None,
};

const LOW_OFF: Thresholds = Thresholds {
    low_free: Some(0),
    critical_free: None,
};

const BOTH_OFF: Thresholds = Thresholds {
    low_free: Some(0),
    critical_free: Some(0),
};

/// Covers: row:RT-03, setting:storage.low_free, setting:storage.critical_free
#[rstest]
// On 100 GiB the fixed sizes are the smaller: `low` below 5 GiB, `critical`
// below 1 GiB.
#[case::large_disk_with_room_is_normal(space(100 * GIB, 6 * GIB), DEFAULTS, StorageLevel::Normal)]
#[case::large_disk_at_five_gib_is_normal(space(100 * GIB, 5 * GIB), DEFAULTS, StorageLevel::Normal)]
#[case::large_disk_under_five_gib_is_low(space(100 * GIB, 5 * GIB - 1), DEFAULTS, StorageLevel::Low)]
#[case::large_disk_under_one_gib_is_critical(space(100 * GIB, GIB - 1), DEFAULTS, StorageLevel::Critical)]
// On 16 GiB the percentages are the smaller: `low` below 1.6 GiB (10 %),
// `critical` below 0.32 GiB (2 %), so 2 GiB free still runs, where a fixed
// 5 GiB would stop it.
#[case::small_disk_with_two_gib_is_normal(space(16 * GIB, 2 * GIB), DEFAULTS, StorageLevel::Normal)]
#[case::small_disk_under_ten_percent_is_low(space(16 * GIB, 1_717_986_917), DEFAULTS, StorageLevel::Low)]
#[case::small_disk_at_ten_percent_is_normal(space(16 * GIB, 1_717_986_918), DEFAULTS, StorageLevel::Normal)]
#[case::small_disk_half_a_gib_is_low(space(16 * GIB, GIB / 2), DEFAULTS, StorageLevel::Low)]
#[case::small_disk_under_two_percent_is_critical(space(16 * GIB, 343_597_382), DEFAULTS, StorageLevel::Critical)]
// Free inodes go by the same percentages, where the filesystem reports them.
#[case::inodes_at_ten_percent_are_normal(inodes(100), DEFAULTS, StorageLevel::Normal)]
#[case::inodes_under_ten_percent_are_low(inodes(99), DEFAULTS, StorageLevel::Low)]
#[case::inodes_under_two_percent_are_critical(inodes(19), DEFAULTS, StorageLevel::Critical)]
// Set sizes replace the defaults.
#[case::a_set_critical_size_beats_its_default(
    space(100 * GIB, 2 * GIB),
    Thresholds { low_free: None, critical_free: Some(3 * GIB) },
    StorageLevel::Critical
)]
#[case::a_set_low_size_beats_its_default(
    space(100 * GIB, 7 * GIB),
    Thresholds { low_free: Some(8 * GIB), critical_free: None },
    StorageLevel::Low
)]
// `0` is off, for space and inodes alike, and leaves the other level on.
#[case::low_off_leaves_little_space_normal(space(100 * GIB, 2 * GIB), LOW_OFF, StorageLevel::Normal)]
#[case::low_off_leaves_few_inodes_normal(inodes(50), LOW_OFF, StorageLevel::Normal)]
#[case::low_off_still_stops_at_critical(space(100 * GIB, GIB / 2), LOW_OFF, StorageLevel::Critical)]
#[case::both_off_never_stop(space(100 * GIB, 0), BOTH_OFF, StorageLevel::Normal)]
fn a_filesystems_level_follows_its_readings_and_the_two_settings(
    #[case] reading: FilesystemReading,
    #[case] thresholds: Thresholds,
    #[case] expected: StorageLevel,
) {
    assert_eq!(level(&reading, &thresholds), expected);
}

fn reading() -> impl Strategy<Value = FilesystemReading> {
    (
        1_u64..=200 * GIB,
        any::<u64>(),
        proptest::option::of((1_u64..=1_000_000, any::<u64>())),
    )
        .prop_map(|(size, free, inodes)| FilesystemReading {
            size,
            free: free % (size + 1),
            inodes: inodes.map(|(total, free)| FilesystemInodes {
                total,
                free: free % (total + 1),
            }),
        })
}

fn thresholds() -> impl Strategy<Value = Thresholds> {
    let size = proptest::option::of(prop_oneof![Just(0_u64), 0..=10 * GIB]);
    (size.clone(), size).prop_map(|(low_free, critical_free)| Thresholds {
        low_free,
        critical_free,
    })
}

proptest! {
    /// More free space never makes a filesystem's level worse, so a level
    /// drops back at the first sample that finds the space again.
    ///
    /// Covers: row:RT-03
    #[test]
    fn more_room_never_makes_the_level_worse(
        reading in reading(),
        thresholds in thresholds(),
        more in any::<u64>(),
    ) {
        let roomier = FilesystemReading {
            free: reading.free + more % (reading.size - reading.free + 1),
            ..reading
        };
        prop_assert!(level(&roomier, &thresholds) <= level(&reading, &thresholds));
    }

    /// A filesystem with both levels off is always `normal`.
    ///
    /// Covers: row:RT-03
    #[test]
    fn with_both_levels_off_a_filesystem_is_always_normal(reading in reading()) {
        prop_assert_eq!(level(&reading, &BOTH_OFF), StorageLevel::Normal);
    }

    /// Under the defaults, free space makes a filesystem `critical` only
    /// below both 1 GiB and 2 % of its size.
    ///
    /// Covers: row:RT-03
    #[test]
    fn under_the_defaults_a_filesystem_with_its_free_space_is_critical_only_below_both_values(
        size in 1_u64..=200 * GIB,
        free in any::<u64>(),
    ) {
        let reading = space(size, free % (size + 1));
        let critical = reading.free < GIB.min(size / 50);
        prop_assert_eq!(level(&reading, &DEFAULTS) == StorageLevel::Critical, critical);
    }
}
