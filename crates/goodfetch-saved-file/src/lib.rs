//! The operating-system part of a Saved file (ADR 0116), which is written
//! whole or not at all: the hidden temporary file a save writes beside its
//! path, and the reason and OS code a failed write reports. Each side keeps
//! its own writer and its own errors over it: the engine's for a kept file,
//! Collie's client crate's for the bytes a Daemon streams.
//!
//! Its own crate because the compiler must keep one implementation on both
//! sides of a dependency direction (STYLE.md, reason 2): the engine and
//! Collie's client crate each save a file, and the client never links the
//! engine.

mod failure;
mod partial;
mod path;

pub use failure::{blames_path, os_code, reason_of, unix_code};
pub use partial::{create_hidden, unhide};
pub use path::{PathIssue, PathShape, check_relative_path, check_save_path, resolve_in_folder};
