//! A Page's worlds (ADR 0146): the isolated worlds the engine keeps in the
//! Page's documents, a field of the value its owner holds. Today that is
//! the guard's world in the main frame's current document, made on first
//! use in each document and found there by every later Gesture's check and
//! by a navigation's encoding read. #1892's registry of each frame's world
//! and #1893's kept script handle are later fields of it.

use std::sync::Arc;

use tokio::sync::watch;

/// The worlds the engine keeps in one Page's documents; its clones share
/// them.
#[derive(Clone, Default)]
pub(crate) struct PageWorlds {
    /// The context of the guard's world in the main frame's current
    /// document, once one was made there.
    guard: Arc<watch::Sender<Option<i64>>>,
}

impl PageWorlds {
    /// The context of the guard's world in the main frame's current
    /// document, when one is kept.
    pub(crate) fn guard(&self) -> Option<i64> {
        *self.guard.borrow()
    }

    /// Keeps `context` as the guard's world in the main frame's current
    /// document.
    pub(crate) fn keep_guard(&self, context: i64) {
        self.guard.send_replace(Some(context));
    }

    /// Forgets the guard's world `context`, whose document went, unless
    /// another call has kept the next document's since.
    pub(crate) fn forget_guard(&self, context: i64) {
        self.guard.send_if_modified(|kept| {
            let gone = *kept == Some(context);
            if gone {
                *kept = None;
            }
            gone
        });
    }
}
