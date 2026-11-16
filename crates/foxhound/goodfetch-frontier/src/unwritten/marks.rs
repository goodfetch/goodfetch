//! Whether the file holds a trap mark, as memory knows it (#2169), so an
//! admission of a link seen again clears one only once one may be there.
//!
//! The crawl's opening reads it once, on the writer. A writer job that
//! marks a link says so; nothing makes memory forget a mark, since a job
//! whose work fails, or a mark the writer deletes, leaves at most a clear
//! that finds nothing. Until the opening reads it, each probe reads the
//! file as before. Memory holds it at every budget: it takes no bytes.

use super::Holding;

/// What memory knows of the file's trap marks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Marks {
    /// Not read yet: a probe reads the file.
    #[default]
    Unread,
    /// The file holds none.
    Unmarked,
    /// The file may hold one: a probe reads nothing and answers it may.
    Marked,
}

impl Holding<'_> {
    /// Whether the file may hold a trap mark: none while memory knows it
    /// holds none, one while memory knows it may; otherwise `read`'s
    /// answer from the file, which memory keeps.
    ///
    /// # Errors
    ///
    /// When `read` fails.
    pub(crate) fn marked_or(
        &mut self,
        read: impl FnOnce() -> rusqlite::Result<bool>,
    ) -> rusqlite::Result<bool> {
        match self.held.marks {
            Marks::Unmarked => return Ok(false),
            Marks::Marked => return Ok(true),
            Marks::Unread => {}
        }
        self.reads_state();
        let marked = read()?;
        self.held.marks = if marked {
            Marks::Marked
        } else {
            Marks::Unmarked
        };
        Ok(marked)
    }

    /// Notes that the file may hold a trap mark.
    pub(crate) fn mark(&mut self) {
        self.held.marks = Marks::Marked;
    }
}
