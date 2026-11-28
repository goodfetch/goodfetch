//! Syndication feed files (RSS 2.0, RSS 1.0, Atom 1.0), plain XML or gzip,
//! as sites serve them (ADR 0067, ADR 0139).
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    goodfetch_frontier::syndication::fuzz_syndication(data);
});
