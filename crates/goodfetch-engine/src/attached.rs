//! An Attached engine's lifetime on the machine (ADRs 0024 and 0031): the
//! files of its private endpoint, and the watch that ends it with the
//! Client that started it, even when that Client dies without closing its
//! stdin.
//!
//! The private endpoint lives in a folder of its own in the OS's temporary
//! folder, mode `0700`, removed when the engine stops: its socket, mode
//! `0600`, on macOS and Linux (loopback TCP on Windows, whose port the
//! engine reports), and the file holding its token, mode `0600`, which
//! every `hello` there carries, as a Daemon's does. The Client of the
//! engine's stdio is told both in the reply to its `hello`, and the Run
//! index names them as the owner of the engine's live Runs, where its
//! `workers=N` handler processes and a Daemon forwarding a live action
//! connect.
//!
//! The parent's death ends the engine on every OS: on Linux the kernel
//! kills it (`PR_SET_PDEATHSIG`), on Windows the kernel kills it when its
//! parent's handle to a kill-on-close Job Object holding it closes, and on
//! macOS a kqueue `NOTE_EXIT` watch on the parent tells the engine, which
//! stops as it does at the end of its stdin.

mod files;
mod parent;

pub(crate) use files::{PrivateFiles, read_token};
pub(crate) use parent::ParentWatch;
