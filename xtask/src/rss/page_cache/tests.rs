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

    // A file none of whose pages has been in the page cache: its length is
    // set, nothing is written or read. It stays open, so the kernel keeps its
    // inode, and with it the shadow entries of its evicted pages.
    let never_read = home.path().join("never-read");
    let held = std::fs::File::create(&never_read).expect("creating the file");
    held.set_len(256 * 1024).expect("sizing the file");
    let (before, pages) = cache_stat(&held);
    assert_eq!(
        (before.cache, before.evicted),
        (0, 0),
        "no page of a file never read is cached or evicted: {before:?}"
    );
    PageCache::Cached
        .prepare(&never_read)
        .expect("the file is read through");
    // Memory pressure may evict a page as soon as it is read (issue 3301),
    // and the kernel leaves a shadow entry for each page it evicts: a page
    // cached or evicted now was read in. Only the shrinker of shadow entries
    // could hide one, and it frees the oldest nodes of them first, once the
    // memory group holds more than its pages allow.
    let (read, _) = cache_stat(&held);
    assert_eq!(
        read.cache + read.evicted,
        pages,
        "cached: every page was read into the page cache: {read:?}"
    );

    let written = home.path().join("written");
    let taken = pages_reclaim_or_compaction_took();
    std::fs::write(&written, vec![0x5a; 256 * 1024]).expect("writing the file");
    PageCache::Dropped
        .prepare(&written)
        .expect("the file is dropped");
    let (dropped, pages) = cache_stat(&std::fs::File::open(&written).expect("opening the file"));
    assert_eq!(
        (dropped.dirty, dropped.writeback),
        (0, 0),
        "dropped: every page was written back: {dropped:?}"
    );
    // The advice frees every clean page no other task holds. Reclaim may
    // hold one at that moment and evict it just after, or compaction move
    // it (issue 3301: all 64 pages, one folio, cached or evicted right after
    // the drop). Each counts the pages it takes as it takes them, so while
    // the count stays the same, neither took one.
    if pages_reclaim_or_compaction_took() == taken {
        assert_eq!(
            (dropped.cache, dropped.evicted),
            (0, 0),
            "dropped: none of its {pages} pages is in the page cache, nor was evicted from it"
        );
    }
}

/// The pages reclaim and compaction have taken off the kernel's page lists
/// since boot, as `/proc/vmstat` counts them. Reclaim counts them in the same
/// locked section that takes them (`pgscan_*`). Compaction counts them when
/// its scan of a block stops (`compact_isolated`), which it does as soon as it
/// holds 32 pages: for the file's one 64-page folio, a few instructions after
/// taking it, and only a preemption in between could delay the count past the
/// test's reading. Compaction also holds a page for an instant without taking
/// it, which the advice outlasts: it tries such a page again after draining
/// every CPU's batches of pages.
#[cfg(target_os = "linux")]
fn pages_reclaim_or_compaction_took() -> u64 {
    std::fs::read_to_string("/proc/vmstat")
        .expect("reading the kernel's memory counts")
        .lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(name, _)| name.starts_with("pgscan") || *name == "compact_isolated")
        .map(|(_, count)| count.parse::<u64>().expect("a count"))
        .sum()
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

/// What `cachestat` counts of `file`'s pages, and how many pages it has.
#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "the page cache's counts go through the OS's C interface: sysconf and cachestat"
)]
fn cache_stat(file: &std::fs::File) -> (CacheStat, u64) {
    use std::os::fd::AsRawFd as _;

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
