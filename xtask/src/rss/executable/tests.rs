use std::path::Path;

use super::file_backed_bytes;

/// A trimmed `/proc/<pid>/smaps` of a dynamically linked Foxhound: its own
/// code and data, a shared library, the heap and an anonymous mapping.
const SMAPS: &str = "\
5581c0a00000-5581c2400000 r--p 00000000 08:01 1234567                    /opt/goodfetch/bin/goodfetch-foxhound
Size:              26624 kB
Rss:                8192 kB
Pss:                8192 kB
Anonymous:             0 kB
VmFlags: rd mr mw me sd
5581c2400000-5581c5000000 r-xp 01a00000 08:01 1234567                    /opt/goodfetch/bin/goodfetch-foxhound
Size:              45056 kB
Rss:               12288 kB
Anonymous:             0 kB
5581c5000000-5581c5200000 rw-p 04600000 08:01 1234567                    /opt/goodfetch/bin/goodfetch-foxhound
Size:               2048 kB
Rss:                1024 kB
Anonymous:           768 kB
5581c6000000-5581c8000000 rw-p 00000000 00:00 0                          [heap]
Size:              32768 kB
Rss:               20480 kB
Anonymous:         20480 kB
7f1a2b000000-7f1a2b200000 r-xp 00000000 08:01 7654321                    /usr/lib/x86_64-linux-gnu/libc.so.6
Size:               2048 kB
Rss:                1536 kB
Anonymous:             0 kB
7f1a2c000000-7f1a2c100000 rw-p 00000000 00:00 0
Size:               1024 kB
Rss:                 512 kB
Anonymous:           512 kB
";

#[test]
fn only_the_executables_own_file_backed_pages_count() {
    let bytes = file_backed_bytes(SMAPS, Path::new("/opt/goodfetch/bin/goodfetch-foxhound"));

    // 8,192 + 12,288 + (1,024 - 768 written) KiB; not the heap, libc or
    // the anonymous mapping.
    assert_eq!(bytes, (8192 + 12288 + 256) * 1024);
}

#[test]
fn another_executables_mappings_count_nothing() {
    let bytes = file_backed_bytes(SMAPS, Path::new("/opt/goodfetch/bin/goodfetch-collie"));

    assert_eq!(bytes, 0);
}

#[test]
fn a_path_with_spaces_is_read_whole() {
    let smaps = "\
00400000-00600000 r-xp 00000000 08:01 42                                 /home/a user/goodfetch foxhound
Rss:                 100 kB
Anonymous:             0 kB
";

    let bytes = file_backed_bytes(smaps, Path::new("/home/a user/goodfetch foxhound"));

    assert_eq!(bytes, 100 * 1024);
}
