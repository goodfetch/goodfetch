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
    let (dropped, pages) = cache_stat(&file);
    assert_eq!(
        (dropped.cache, dropped.evicted),
        (0, 0),
        "dropped: none of its {pages} pages is in the page cache, nor was evicted from it"
    );
    PageCache::Cached
        .prepare(&file)
        .expect("the file is read through");
    // The kernel may evict clean pages under memory pressure as soon as they
    // are read (issue 3301), and it leaves a shadow entry for each such page,
    // which the drop above cleared: a page cached or evicted since was read in.
    // Only a shrinker of shadow entries could hide one, and it frees a node
    // of them only once every page of it is evicted and the memory group
    // holds more shadow entries than its pages allow.
    let (read, pages) = cache_stat(&file);
    assert_eq!(
        read.cache + read.evicted,
        pages,
        "cached: every page was read into the page cache: {read:?}"
    );
}

/// `cachestat`'s number in the table the architectures share from 424 on,
/// which `x86_64` and `aarch64`, this repository's Linux targets, use; MIPS and
/// Alpha number it otherwise.
#[cfg(target_os = "linux")]
const SYS_CACHESTAT: libc::c_long = 451;

/// The range `cachestat` reads: the whole file, as a length of 0 says.
#[cfg(target_os = "linux")]
#[repr(C)]
struct CacheStatRange {
    off: u64,
    len: u64,
}

/// A file's pages in the page cache, as `cachestat` counts them (Linux 6.5):
/// the kernel's `struct cachestat`, its `nr_` fields in their order.
#[cfg(target_os = "linux")]
#[repr(C)]
#[derive(Debug, Default)]
struct CacheStat {
    cache: u64,
    dirty: u64,
    writeback: u64,
    /// Pages evicted from the page cache whose shadow entries remain.
    evicted: u64,
    recently_evicted: u64,
}

/// What `cachestat` counts of `path`'s pages, and how many pages it has.
#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "the page cache's counts go through the OS's C interface: sysconf and cachestat"
)]
fn cache_stat(path: &Path) -> (CacheStat, u64) {
    use std::os::fd::AsRawFd as _;

    let file = std::fs::File::open(path).expect("opening the file");
    let length = file.metadata().expect("its size").len();
    // SAFETY: sysconf reads a constant of the OS.
    let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).expect("a page size");
    let range = CacheStatRange { off: 0, len: 0 };
    let mut stat = CacheStat::default();
    // SAFETY: the descriptor is `file`'s, open for the whole call; `range` and
    // `stat` are the kernel's `struct cachestat_range` and `struct cachestat`,
    // read and written only during it.
    let result = unsafe {
        libc::syscall(
            SYS_CACHESTAT,
            file.as_raw_fd(),
            &raw const range,
            &raw mut stat,
            0_u32,
        )
    };
    assert_eq!(
        result,
        0,
        "reading the file's page cache counts with cachestat, which needs Linux 6.5: {}",
        std::io::Error::last_os_error()
    );
    (stat, length.div_ceil(page))
}
