//! Time, importing a module it may not.

use crate::retry;

pub(crate) fn now() -> u64 {
    retry::retry()
}
