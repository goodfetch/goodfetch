//! The ledger's filesystem from a mount table (spec #213's Seam 4, pure).

use std::path::Path;

use rstest::rstest;

use super::temporary_in;

/// A container's mount table: an overlay root, a tmpfs `/tmp`, a
/// volume over part of it, and a home whose path holds a space.
const MOUNTS: &str = "overlay / overlay rw,relatime 0 0\n\
    proc /proc proc rw,nosuid 0 0\n\
    tmpfs /tmp tmpfs rw,nosuid,nodev 0 0\n\
    /dev/sda1 /tmp/kept ext4 rw,relatime 0 0\n\
    none /home/some\\040one ramfs rw 0 0\n";

/// Covers: story:3-78
#[rstest]
#[case::on_the_root("/root/.local/share/goodfetch/usage-ledger.sqlite", None)]
#[case::on_a_tmpfs("/tmp/goodfetch/usage-ledger.sqlite", Some("tmpfs"))]
#[case::on_a_volume_below_a_tmpfs("/tmp/kept/goodfetch/usage-ledger.sqlite", None)]
#[case::on_a_ramfs_whose_path_holds_a_space("/home/some one/usage-ledger.sqlite", Some("ramfs"))]
#[case::beside_a_mount_point_of_a_longer_name("/tmpfoo/usage-ledger.sqlite", None)]
fn the_ledgers_filesystem_is_the_deepest_mount_above_it(
    #[case] path: &str,
    #[case] temporary: Option<&str>,
) {
    assert_eq!(temporary_in(MOUNTS, Path::new(path)).as_deref(), temporary);
}

#[test]
fn a_later_mount_over_the_same_point_hides_the_earlier_one() {
    let mounts = "tmpfs /data tmpfs rw 0 0\n/dev/sdb1 /data xfs rw 0 0\n";

    assert_eq!(temporary_in(mounts, Path::new("/data/ledger")), None);
}
