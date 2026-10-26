//! The bytes a writer that owns its output writes, shared with the
//! encoder that hands them on.

use std::io::{self, Write};
use std::sync::Arc;

/// A file's bytes as a writer that owns its output writes them, which the
/// encoder takes after each row or batch: the ZIP writer owns its writer
/// while a deflated entry is open, as Arrow's and Parquet's own theirs, so
/// the bytes are shared with it; one task writes and takes them in turn.
#[derive(Clone, Default)]
pub(super) struct Written(Arc<Lock>);

#[expect(
    clippy::disallowed_types,
    reason = "never contended: one task writes into it and takes from it in turn; a plain lock makes the encoder `Send`, which the Export's call needs (ADR 0060's loom has no model of it to check)"
)]
type Lock = std::sync::Mutex<Vec<u8>>;

impl Written {
    pub(super) fn take(&self, out: &mut Vec<u8>) {
        let mut bytes = self
            .0
            .lock()
            .expect("invariant: no writer of an Export's bytes panics holding them");
        out.append(&mut bytes);
    }
}

impl Write for Written {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("invariant: no writer of an Export's bytes panics holding them")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
