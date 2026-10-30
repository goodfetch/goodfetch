//! A store's file met at its switch to WAL (#1773, #1774): another engine
//! holds a write on the new file, still in its first journal mode, while
//! this one opens it. SQLite answers that switch busy at once, without
//! calling the busy handler, so each store's own tests open their file
//! through [`opened_beside_a_write`] and expect it to wait the write out.
#![expect(
    clippy::disallowed_methods,
    reason = "the other engine's write runs on a thread of its own, as in its process, and ends after a pause"
)]

use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

/// How long the other engine holds its write on the new file.
const HELD: Duration = Duration::from_millis(300);

/// Answers what `open` answers, run while another connection holds a write
/// transaction on a new file at `path` for [`HELD`].
pub(crate) fn opened_beside_a_write<T>(path: &Path, open: impl FnOnce() -> T) -> T {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).unwrap();
    }
    assert!(!path.exists(), "{} is not a new file", path.display());
    let other = Connection::open(path).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    let ends = std::thread::spawn(move || {
        std::thread::sleep(HELD);
        other.execute_batch("COMMIT").unwrap();
    });
    let opened = open();
    ends.join().unwrap();
    opened
}
