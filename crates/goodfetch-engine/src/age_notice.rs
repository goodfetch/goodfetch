//! The age notice (ADR 0030): an executable more than three months past
//! its own release date prints one line a day on stderr, saying that newer
//! versions carry security fixes and a current Chrome and giving the
//! upgrade command of its channel. It compares dates only, in UTC, and
//! makes no network call; `version.age_notice = false`, in user
//! configuration or an Operator policy, turns it off. The day it was last
//! shown is kept per user in Goodfetch home, outside the per-major folders.
//!
//! An executable's release date is what its release build records; a build
//! that is no release has none, and never prints the notice. The caller
//! writes the line, on stderr only, never on a protocol stream or in
//! `--json` output.
#![expect(
    clippy::disallowed_methods,
    reason = "the notice is read and recorded on the command's own thread, before any runtime starts"
)]

use std::io;
use std::path::PathBuf;
use std::time::SystemTime;

use goodfetch_protocol::versions::VERSION_AGE_NOTICE;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan as _};

use crate::home::Home;
use crate::product::Identity;
use crate::settings::SettingsFiles;

/// The file, in Goodfetch home, of the day the notice was last shown.
const SHOWN_FILE: &str = "age-notice";

/// Whether an executable released on `released` is more than three months
/// old at the wall time `wall`, by the calendar in UTC.
pub fn is_old(released: Date, wall: SystemTime) -> bool {
    let Ok(now) = Timestamp::try_from(wall) else {
        return false;
    };
    let today = now.to_zoned(TimeZone::UTC).date();
    released
        .checked_add(3.months())
        .is_ok_and(|limit| today > limit)
}

/// The age notice of one executable in one Goodfetch home.
#[derive(Clone, Debug)]
pub struct AgeNotice {
    shown: PathBuf,
    settings: SettingsFiles,
    identity: Identity,
}

impl AgeNotice {
    /// The notice of the executable `identity` describes, in `home`.
    pub fn new(home: &Home, identity: Identity) -> Self {
        Self {
            shown: home.root().join(SHOWN_FILE),
            settings: SettingsFiles::of(home, &identity.product_version),
            identity,
        }
    }

    /// The line to print at the wall time `wall` for an executable released
    /// on `released`, if it is due: the executable is old, the notice was
    /// not shown on that day, and `version.age_notice` does not turn it
    /// off. A due notice is recorded as shown that day. Blocking.
    ///
    /// # Errors
    ///
    /// When the day it was shown cannot be recorded.
    pub fn due(&self, released: Date, wall: SystemTime) -> io::Result<Option<String>> {
        if !is_old(released, wall) {
            return Ok(None);
        }
        let today = Timestamp::try_from(wall)
            .map_err(io::Error::other)?
            .to_zoned(TimeZone::UTC)
            .date()
            .to_string();
        match std::fs::read_to_string(&self.shown) {
            Ok(shown) if shown.trim() == today => return Ok(None),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if !self.turned_on() {
            return Ok(None);
        }
        if let Some(folder) = self.shown.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(&self.shown, format!("{today}\n"))?;
        Ok(Some(self.line(released)))
    }

    /// Whether the machine's layers leave the notice on: settings files
    /// that do not read leave it off, since the command reports them.
    fn turned_on(&self) -> bool {
        let product = self.identity.product;
        self.settings
            .machine_settings(product)
            .is_ok_and(|resolved| resolved.get(&VERSION_AGE_NOTICE))
    }

    fn line(&self, released: Date) -> String {
        format!(
            "goodfetch-{} {} was released on {released}, more than three months ago; newer versions carry security fixes and a current Chrome. Upgrade with `{}`, or turn this notice off by setting `{}` to `false` in user configuration.",
            self.identity.product.as_str(),
            self.identity.product_version,
            self.identity.channel.install_command(self.identity.product),
            VERSION_AGE_NOTICE.name,
        )
    }
}

#[cfg(test)]
mod tests;
