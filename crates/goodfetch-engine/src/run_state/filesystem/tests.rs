use std::path::Path;

use goodfetch_protocol::run::RunStateFilesystem;
use rstest::rstest;

use super::{Magic, Mount, check, magic, mount_of, object_storage_tool, unescaped};

/// Covers: story:3-33
#[rstest]
#[case::nfs(0x6969, Magic::Refused(RunStateFilesystem::Nfs))]
#[case::smbfs(0x517b, Magic::Refused(RunStateFilesystem::Smb))]
#[case::cifs(0xff53_4d42, Magic::Refused(RunStateFilesystem::Smb))]
#[case::smb2(0xfe53_4d42, Magic::Refused(RunStateFilesystem::Smb))]
#[case::fuse(0x6573_5546, Magic::Fuse)]
#[case::ext4(0xef53, Magic::Kept)]
#[case::xfs(0x5846_5342, Magic::Kept)]
#[case::tmpfs(0x0102_1994, Magic::Kept)]
#[case::overlay(0x794c_7630, Magic::Kept)]
fn a_filesystems_magic_number_says_whether_it_is_refused(#[case] f_type: u32, #[case] said: Magic) {
    assert_eq!(magic(f_type), said);
}

const MOUNTINFO: &str = "\
22 1 252:1 / / rw,relatime shared:1 - ext4 /dev/vda1 rw
40 22 0:35 / /mnt/bucket rw,nosuid,nodev,relatime shared:20 - fuse.s3fs s3fs rw,user_id=0,group_id=0
41 22 0:36 / /mnt/mountpoint rw,nosuid,nodev,relatime shared:21 - fuse mountpoint-s3 rw,user_id=0,group_id=0
42 22 0:37 / /mnt/sshfs rw,nosuid,nodev,relatime shared:22 - fuse.sshfs user@host:/srv rw,user_id=0,group_id=0
43 22 0:38 / /mnt/with\\040space rw,relatime shared:23 - fuse.gcsfuse my-bucket rw,user_id=0,group_id=0
44 22 0:39 / /mnt/rclone rw,relatime shared:24 - fuse.rclone remote:bucket rw,user_id=0,group_id=0
45 40 0:40 / /mnt/bucket/local rw,relatime shared:25 - ext4 /dev/vdb rw
46 22 0:41 / /mnt/stacked rw,relatime shared:26 - ext4 /dev/vdc rw
47 22 0:42 / /mnt/stacked rw,relatime shared:27 - fuse /usr/bin/goofys#bucket rw,user_id=0,group_id=0
";

/// Covers: story:3-33
#[rstest]
#[case::subtype("/mnt/bucket/runs/r1", Some("s3fs"))]
#[case::source_names_the_tool("/mnt/mountpoint/runs", Some("mountpoint-s3"))]
#[case::escaped_mount_point("/mnt/with space/runs", Some("gcsfuse"))]
#[case::remote_source("/mnt/rclone", Some("rclone"))]
#[case::latest_mount_on_one_point("/mnt/stacked/runs", Some("goofys"))]
#[case::fuse_of_no_object_store("/mnt/sshfs/runs", None)]
#[case::disk_inside_a_bucket_mount("/mnt/bucket/local/runs", None)]
#[case::root_disk("/home/operator/runs", None)]
fn a_fuse_folders_tool_is_read_from_the_mount_it_is_on(
    #[case] folder: &str,
    #[case] tool: Option<&str>,
) {
    let mount = mount_of(MOUNTINFO, Path::new(folder)).expect("the root mount holds every folder");
    assert_eq!(object_storage_tool(&mount), tool);
}

#[test]
fn a_folder_beside_a_mount_point_with_its_name_as_prefix_is_not_on_it() {
    let mount = mount_of(MOUNTINFO, Path::new("/mnt/bucket-two/runs")).unwrap();
    assert_eq!(
        mount,
        Mount {
            fstype: "ext4",
            source: "/dev/vda1"
        }
    );
}

#[rstest]
#[case::space("/mnt/a\\040b", "/mnt/a b")]
#[case::tab_and_newline("/a\\011b\\012c", "/a\tb\nc")]
#[case::backslash("/a\\134b", "/a\\b")]
#[case::not_an_escape("/a\\xb", "/a\\xb")]
#[case::short_escape_at_the_end("/a\\04", "/a\\04")]
fn a_mount_point_is_read_with_its_escapes_undone(#[case] escaped: &str, #[case] point: &str) {
    assert_eq!(unescaped(escaped), point);
}

#[test]
fn a_run_file_on_a_local_folder_is_kept() {
    let folder = tempfile::tempdir().unwrap();
    assert!(check(&folder.path().join("run.sqlite")).is_ok());
}
