use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use goodfetch_protocol::hostile_input::PathHazard;
use proptest::prelude::*;
use rstest::rstest;
use tar::{EntryType, Header};
use tempfile::TempDir;

use super::{BYTE_LIMIT, BundleError, ENTRY_LIMIT, Refusal, fuzz_plugin_bundle, unpack};
use crate::plugin_oci::Compression;

/// One entry of an archive a test builds, its path and link target
/// written as given, unchecked.
#[derive(Clone, Debug)]
enum Entry {
    File(&'static str, &'static [u8], u32),
    Folder(&'static str),
    Symlink(&'static str, &'static str),
    HardLink(&'static str, &'static str),
    Other(&'static str, EntryType),
}

/// Writes `text` into the header field `field`, as a hostile archive
/// would, past the checks `tar::Builder` makes.
fn raw(field: &mut [u8; 100], text: &str) {
    field.fill(0);
    field[..text.len()].copy_from_slice(text.as_bytes());
}

/// A tar archive of `entries`, in order.
fn archive(entries: &[Entry]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for entry in entries {
        let mut header = Header::new_gnu();
        header.set_mode(0o644);
        let (path, kind, data, link): (&str, EntryType, &[u8], Option<&str>) = match entry {
            Entry::File(path, data, mode) => {
                header.set_mode(*mode);
                (path, EntryType::Regular, data, None)
            }
            Entry::Folder(path) => (path, EntryType::Directory, b"", None),
            Entry::Symlink(path, target) => (path, EntryType::Symlink, b"", Some(target)),
            Entry::HardLink(path, target) => (path, EntryType::Link, b"", Some(target)),
            Entry::Other(path, kind) => (path, *kind, b"", None),
        };
        raw(&mut header.as_old_mut().name, path);
        if let Some(link) = link {
            raw(&mut header.as_old_mut().linkname, link);
        }
        header.set_entry_type(kind);
        header.set_size(u64::try_from(data.len()).unwrap());
        header.set_cksum();
        builder.append(&header, data).unwrap();
    }
    builder.into_inner().unwrap()
}

/// A new folder `bundle` in a new temporary folder, which a test checks
/// holds nothing else after an unpack.
fn folders() -> (TempDir, PathBuf) {
    let outer = tempfile::tempdir().unwrap();
    let into = outer.path().join("bundle");
    fs::create_dir(&into).unwrap();
    (outer, into)
}

/// Unpacks `entries`, plain, into a new folder; asserts nothing was
/// written beside it.
fn unpacked(entries: &[Entry]) -> (TempDir, PathBuf, Result<(), BundleError>) {
    let (outer, into) = folders();
    let result = unpack(archive(entries).as_slice(), Compression::None, &into);
    let beside = fs::read_dir(outer.path()).unwrap().count();
    assert_eq!(beside, 1, "the unpack wrote beside its folder");
    (outer, into, result)
}

/// The permissions of the file at `path`.
#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

/// A bundle's files, folders and links inside it are made, a leading `./`
/// dropped; an executable keeps `0755` and loses its setuid bit, and
/// nothing else keeps an execute bit.
///
/// Covers: story:7-5, story:7-81
#[test]
fn a_bundles_files_folders_and_links_are_unpacked() {
    let mut entries = vec![
        Entry::Folder("./"),
        Entry::File("./goodfetch-plugin.json", b"{}", 0o644),
        Entry::Folder("bin/"),
        Entry::File("bin/kettle", b"#!/bin/sh\n", 0o4755),
        Entry::File("lib/deep/data.txt", b"data", 0o666),
        Entry::Folder("lib/deep"),
        Entry::HardLink("lib/copy.txt", "lib/deep/data.txt"),
    ];
    if cfg!(unix) {
        entries.push(Entry::Symlink("lib/link.txt", "deep/./data.txt"));
        entries.push(Entry::Symlink("bin/up", "../lib/deep"));
    }

    let (_outer, into, result) = unpacked(&entries);

    result.unwrap();
    assert_eq!(fs::read(into.join("goodfetch-plugin.json")).unwrap(), b"{}");
    assert_eq!(fs::read(into.join("bin/kettle")).unwrap(), b"#!/bin/sh\n");
    assert_eq!(fs::read(into.join("lib/copy.txt")).unwrap(), b"data");
    #[cfg(unix)]
    {
        assert_eq!(mode(&into.join("bin/kettle")), 0o755);
        assert_eq!(mode(&into.join("lib/deep/data.txt")) & 0o111, 0);
        assert_eq!(fs::read(into.join("lib/link.txt")).unwrap(), b"data");
        assert_eq!(
            fs::read_link(into.join("bin/up")).unwrap(),
            Path::new("../lib/deep")
        );
    }
}

/// A gzip-compressed bundle unpacks as its tar archive does.
///
/// Covers: story:7-5
#[test]
fn a_gzip_compressed_bundle_is_unpacked() {
    let tar = archive(&[Entry::File("a.txt", b"a", 0o644)]);
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&tar).unwrap();
    let gzip = encoder.finish().unwrap();
    let (_outer, into) = folders();

    unpack(gzip.as_slice(), Compression::Gzip, &into).unwrap();

    assert_eq!(fs::read(into.join("a.txt")).unwrap(), b"a");
}

/// An entry whose path climbs out, starts at a root or is unsafe on some
/// OS, a link whose target leaves the bundle or passes through another
/// link, an entry below a link or a file, an entry made twice, a device, a
/// FIFO or a hard link to no earlier file is refused, and nothing is
/// written outside the folder.
///
/// Covers: story:7-81
#[rstest]
#[case::traversal(vec![Entry::File("../evil", b"x", 0o644)], Refusal::Unsafe(PathHazard::ParentTraversal))]
#[case::traversal_at_depth(vec![Entry::File("a/../../evil", b"x", 0o644)], Refusal::Unsafe(PathHazard::ParentTraversal))]
#[case::absolute(vec![Entry::File("/tmp/evil", b"x", 0o644)], Refusal::Unsafe(PathHazard::Absolute))]
#[case::reserved_on_windows(vec![Entry::File("bin/con.txt", b"x", 0o644)], Refusal::Unsafe(PathHazard::ReservedName))]
#[case::backslash(vec![Entry::File("a\\..\\..\\evil", b"x", 0o644)], Refusal::Unportable)]
#[case::the_root_as_a_file(vec![Entry::File("./", b"x", 0o644)], Refusal::Unsafe(PathHazard::DotName))]
#[case::twice(vec![Entry::File("a", b"x", 0o644), Entry::File("a", b"y", 0o644)], Refusal::Duplicate)]
#[case::a_file_over_a_folder(vec![Entry::File("a/b", b"x", 0o644), Entry::File("a", b"y", 0o644)], Refusal::Duplicate)]
#[case::below_a_file(vec![Entry::File("a", b"x", 0o644), Entry::File("a/b", b"y", 0o644)], Refusal::BelowFileOrLink)]
#[case::character_device(vec![Entry::Other("tty", EntryType::Char)], Refusal::Kind("character device".to_owned()))]
#[case::block_device(vec![Entry::Other("disk", EntryType::Block)], Refusal::Kind("block device".to_owned()))]
#[case::fifo(vec![Entry::Other("pipe", EntryType::Fifo)], Refusal::Kind("FIFO".to_owned()))]
#[case::hard_link_to_nothing(vec![Entry::HardLink("a", "b")], Refusal::HardLinkTarget("b".to_owned()))]
#[case::hard_link_out(vec![Entry::HardLink("a", "../../etc/passwd")], Refusal::Unsafe(PathHazard::ParentTraversal))]
#[case::hard_link_to_a_folder(vec![Entry::Folder("d"), Entry::HardLink("a", "d")], Refusal::HardLinkTarget("d".to_owned()))]
fn an_entry_the_unpack_does_not_make_is_refused(
    #[case] entries: Vec<Entry>,
    #[case] refusal: Refusal,
) {
    let (_outer, _into, result) = unpacked(&entries);

    match result {
        Err(BundleError::Entry { refusal: found, .. }) => assert_eq!(found, refusal),
        other => panic!("{other:?}"),
    }
}

/// Covers: story:7-81
#[cfg(unix)]
#[rstest]
#[case::up_and_out(vec![Entry::Symlink("a/l", "../../etc/passwd")], Refusal::LinkLeaves("../../etc/passwd".to_owned()))]
#[case::absolute(vec![Entry::Symlink("l", "/etc/passwd")], Refusal::LinkLeaves("/etc/passwd".to_owned()))]
#[case::the_bundle_itself(vec![Entry::Symlink("l", ".")], Refusal::LinkLeaves(".".to_owned()))]
#[case::through_a_link(vec![Entry::Folder("a"), Entry::Symlink("s", "a"), Entry::Symlink("l", "s/../../x")], Refusal::LinkThroughLink("s/../../x".to_owned()))]
#[case::through_a_later_link(vec![Entry::Symlink("l", "s/x"), Entry::Symlink("s", ".")], Refusal::LinkThroughLink("s/x".to_owned()))]
#[case::below_a_link(vec![Entry::Symlink("s", "a"), Entry::File("s/x", b"x", 0o644)], Refusal::BelowFileOrLink)]
#[case::a_link_over_a_file(vec![Entry::File("a", b"x", 0o644), Entry::Symlink("a", "b")], Refusal::Duplicate)]
fn a_link_that_leaves_the_bundle_is_refused(#[case] entries: Vec<Entry>, #[case] refusal: Refusal) {
    let (_outer, into, result) = unpacked(&entries);

    match result {
        Err(BundleError::Entry { refusal: found, .. }) => assert_eq!(found, refusal),
        other => panic!("{other:?}"),
    }
    let links = fs::read_dir(&into)
        .unwrap()
        .filter(|entry| entry.as_ref().unwrap().file_type().unwrap().is_symlink())
        .count();
    assert_eq!(links, 0, "a link was made before every link was checked");
}

/// An entry that would take the bundle past its limits is refused before
/// it is written.
///
/// Covers: story:7-81
#[test]
fn a_bundle_past_its_limits_is_refused() {
    let mut header = Header::new_gnu();
    header.set_path("huge").unwrap();
    header.set_size(BYTE_LIMIT + 1);
    header.set_cksum();
    let mut huge = header.as_bytes().to_vec();
    huge.extend_from_slice(&[0; 1024]);
    let many = archive(&vec![
        Entry::Folder("d");
        usize::try_from(ENTRY_LIMIT).unwrap() + 1
    ]);

    let (_outer, into) = folders();
    let too_large = unpack(huge.as_slice(), Compression::None, &into);
    let (_outer, into) = folders();
    let too_many = unpack(many.as_slice(), Compression::None, &into);

    assert!(
        matches!(too_large, Err(BundleError::TooLarge)),
        "{too_large:?}"
    );
    assert!(
        matches!(too_many, Err(BundleError::TooManyEntries)),
        "{too_many:?}"
    );
}

/// Bytes that are no tar archive, or no gzip stream when one is declared,
/// are refused.
///
/// Covers: story:7-81
#[rstest]
#[case::not_tar(b"not a tar archive at all, though long enough to hold a header".repeat(10), Compression::None)]
#[case::not_gzip(archive(&[Entry::File("a", b"x", 0o644)]), Compression::Gzip)]
fn bytes_that_are_no_archive_are_refused(#[case] bytes: Vec<u8>, #[case] compression: Compression) {
    let (_outer, into) = folders();

    let refused = unpack(bytes.as_slice(), compression, &into);

    assert!(
        matches!(refused, Err(BundleError::Archive(_))),
        "{refused:?}"
    );
}

/// A path or a link target built from `a`, `b`, `.` and `..`.
fn hostile_path() -> impl Strategy<Value = &'static str> {
    proptest::sample::select(vec![
        "a",
        "b",
        "a/b",
        "./a",
        "a/",
        "..",
        "../a",
        "a/..",
        "a/../..",
        "a/b/../../..",
        ".",
        "/a",
        "b/a",
        "a/./b",
    ])
}

/// An entry of a hostile archive.
fn hostile_entry() -> impl Strategy<Value = Entry> {
    prop_oneof![
        hostile_path().prop_map(|path| Entry::File(path, b"x", 0o644)),
        hostile_path().prop_map(Entry::Folder),
        (hostile_path(), hostile_path()).prop_map(|(path, target)| Entry::Symlink(path, target)),
        (hostile_path(), hostile_path()).prop_map(|(path, target)| Entry::HardLink(path, target)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Whatever entries an archive holds, the unpack writes only inside
    /// its folder and every link it makes resolves inside it.
    ///
    /// Covers: story:7-81
    #[test]
    fn no_archive_writes_outside_its_folder(entries in proptest::collection::vec(hostile_entry(), 1..8)) {
        fuzz_plugin_bundle(&archive(&entries));
    }
}

goodfetch_test_support::fuzz_replay!(
    crate::plugin_bundle::fuzz_plugin_bundle,
    "fuzz/plugin_bundle"
);
