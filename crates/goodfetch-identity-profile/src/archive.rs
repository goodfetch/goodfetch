//! The one file of a population archive, as fingerprint-suite publishes
//! it: a ZIP of a single deflated entry, read from its local header.

use std::io::Read as _;

use flate2::Crc;
use flate2::read::DeflateDecoder;

const LOCAL_HEADER: u32 = 0x0403_4b50;
const DEFLATED: u16 = 8;
/// The entry's sizes follow its data instead of its local header.
const SIZES_AFTER_DATA: u16 = 1 << 3;

/// The bytes of `archive`'s first entry, checked against its CRC-32.
pub(crate) fn only_entry(archive: &[u8]) -> Vec<u8> {
    let u16_at = |at: usize| u16::from_le_bytes([archive[at], archive[at + 1]]);
    let u32_at = |at: usize| u32::from(u16_at(at)) | (u32::from(u16_at(at + 2)) << 16);
    assert!(
        u32_at(0) == LOCAL_HEADER && u16_at(8) == DEFLATED && u16_at(6) & SIZES_AFTER_DATA == 0,
        "invariant: an embedded population archive starts with a deflated entry that states its sizes"
    );
    let size = |at: usize| usize::try_from(u32_at(at)).expect("invariant: a u32 fits a usize");
    let start = 30 + usize::from(u16_at(26)) + usize::from(u16_at(28));
    let mut entry = Vec::with_capacity(size(22));
    DeflateDecoder::new(&archive[start..start + size(18)])
        .read_to_end(&mut entry)
        .expect("invariant: an embedded population archive inflates");
    let mut crc = Crc::new();
    crc.update(&entry);
    assert_eq!(
        crc.sum(),
        u32_at(14),
        "invariant: an embedded population archive matches its CRC-32"
    );
    entry
}
