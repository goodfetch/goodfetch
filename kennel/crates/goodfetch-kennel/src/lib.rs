//! The Kennel's modules that need no graphical toolkit (ADR 0141), so that
//! every host builds and tests them, the display-less Linux gate's
//! included:
//! - `client`: the Daemon's Connection, from its runtime file to `hello`
//!   and the changes it goes through;
//! - `state`: the pure model, from those changes and the settings to the
//!   view the Tray draws;
//! - `settings`: the Kennel's own settings file, read at launch and on
//!   change;
//! - `daemon_start`: the installed Product's `daemon start`;
//! - `home`: the Goodfetch home and the files the Kennel reads in it;
//! - `menu`: the edits that turn a shown menu into the next view's rows in
//!   place, for a Tray whose menu cannot be replaced;
//! - `tasks`: the one tracker every task starts through;
//! - `text`: the Text function, through which every shown string passes;
//! - `window`: the details window's process, a second process of the
//!   Kennel's executable.

pub mod client;
pub mod daemon_start;
pub mod home;
pub mod menu;
pub mod settings;
pub mod state;
pub mod tasks;
pub mod text;
pub mod window;
