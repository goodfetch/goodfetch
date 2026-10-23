//! What loading the embedded Filter lists holds (ADR 0057): they load from
//! their zstd form and the decompressed bytes go once deserialized, so the
//! process's resident memory grows by the engines alone, under the 9.06 MB
//! ADR 0057 measured for the serialized embedded lists. Kept, the
//! decompressed bytes would double that. Each test runs in a process of its
//! own (nextest), so nothing else of the test's grows it.

use goodfetch_clean_up_rules::rules::FilterLists;

/// ADR 0057's figure: the serialized embedded lists, 9.06 MB.
const ADR_0057_SERIALIZED: usize = 9_060_000;

/// Covers: story:5-94
#[test]
fn loading_the_embedded_lists_keeps_resident_memory_under_adr_0057s_figure() {
    let before = resident();
    let lists = FilterLists::embedded().expect("the embedded Filter lists load");
    let after = resident();
    let grown = after.saturating_sub(before);
    assert!(
        grown < ADR_0057_SERIALIZED,
        "loading grew resident memory by {grown} bytes, over {ADR_0057_SERIALIZED}; the lists' serialized size is {}",
        FilterLists::serialized_size()
    );
    drop(lists);
}

/// The process's own resident memory, from `/proc/self/statm`: its
/// resident pages less those its files back, such as the executable's
/// pages that hold the compressed lists, which the OS shares and drops.
#[expect(
    clippy::disallowed_methods,
    reason = "a synchronous test reads its own /proc file"
)]
fn resident() -> usize {
    let statm = std::fs::read_to_string("/proc/self/statm").expect("Linux has /proc/self/statm");
    let fields: Vec<usize> = statm
        .split_whitespace()
        .map(|field| field.parse().expect("statm holds page counts"))
        .collect();
    let (resident, file_backed) = (fields[1], fields[2]);
    // 4 KiB pages on every Linux this runs on (x86-64, aarch64 with 4K pages).
    (resident - file_backed) * 4096
}
