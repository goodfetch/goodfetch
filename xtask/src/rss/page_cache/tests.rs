use super::*;

#[test]
fn a_missing_file_fails_naming_it() {
    let home = tempfile::tempdir().expect("a temporary folder");
    let missing = home.path().join("no-such-executable");
    let result = drop_cached(&missing);
    if DROPS {
        let error = result.expect_err("a missing file is no executable to drop");
        assert!(
            format!("{error:#}").contains("no-such-executable"),
            "the error names the file: {error:#}"
        );
    } else {
        result.expect("this OS drops nothing");
    }
}

#[test]
fn an_existing_file_is_dropped() {
    let home = tempfile::tempdir().expect("a temporary folder");
    let file = home.path().join("executable");
    std::fs::write(&file, b"pages").expect("writing the file");
    drop_cached(&file).expect("the file is dropped");
}

#[cfg(target_os = "linux")]
#[test]
fn each_mode_leaves_the_executable_s_pages_where_it_says() {
    let home = tempfile::tempdir().expect("a temporary folder");
    let file = home.path().join("executable");
    std::fs::write(&file, vec![0x5a; 256 * 1024]).expect("writing the file");
    PageCache::Dropped
        .prepare(&file)
        .expect("the file is dropped");
    let (cached, pages) = cached_pages(&file);
    assert_eq!(
        cached, 0,
        "dropped: none of its {pages} pages is in the page cache"
    );
    PageCache::Cached
        .prepare(&file)
        .expect("the file is read through");
    let (cached, pages) = cached_pages(&file);
    assert_eq!(cached, pages, "cached: every page is in the page cache");
}

/// How many of `path`'s pages are in the page cache, and how many pages it
/// has, read with `mincore` over a mapping of it that faults none in.
#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "the page cache's residency goes through the OS's C interface: mmap and mincore"
)]
fn cached_pages(path: &Path) -> (usize, usize) {
    use std::os::fd::AsRawFd as _;

    let file = std::fs::File::open(path).expect("opening the file");
    let length =
        usize::try_from(file.metadata().expect("its size").len()).expect("a size that fits");
    // SAFETY: sysconf reads a constant of the OS.
    let page = usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).expect("a page size");
    let pages = length.div_ceil(page);
    // SAFETY: a fresh read-only shared mapping of the whole open file, unmapped below.
    let mapping = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            length,
            libc::PROT_READ,
            libc::MAP_SHARED,
            file.as_raw_fd(),
            0,
        )
    };
    assert_ne!(mapping, libc::MAP_FAILED, "mapping the file");
    let mut residency = vec![0_u8; pages];
    // SAFETY: `mapping` spans `length` bytes, and `residency` holds one byte per page of it.
    let error = unsafe { libc::mincore(mapping, length, residency.as_mut_ptr()) };
    // SAFETY: `mapping` and `length` are the mapping made above, used no more.
    unsafe { libc::munmap(mapping, length) };
    assert_eq!(error, 0, "reading the file's residency");
    (
        residency.iter().filter(|byte| *byte & 1 == 1).count(),
        pages,
    )
}
