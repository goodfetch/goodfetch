//! The primitive that has a page show other values than its machine's
//! through the browser's own overrides (ADR 0035's `basic` Fingerprint
//! method): its time zone, its locale, its geolocation and its core count;
//! and the one that has it run a script in each document's main world
//! before the document's own, from its opening (`js_injection`).

use goodfetch_cdp::generated::{emulation, page};
use goodfetch_engine::CallError;

use super::CdpPage;

/// The values a page shows in place of its machine's; each one left out
/// stays the machine's.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Emulated {
    /// An IANA time zone, such as `Europe/Paris`.
    pub(crate) time_zone: Option<String>,
    /// A BCP 47 locale, such as `fr-FR`.
    pub(crate) locale: Option<String>,
    /// Where the page's geolocation says it is.
    pub(crate) geolocation: Option<Geolocation>,
    /// The core count `navigator.hardwareConcurrency` reads.
    pub(crate) core_count: Option<u32>,
}

/// A position a page's geolocation answers with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Geolocation {
    pub(crate) latitude: f64,
    pub(crate) longitude: f64,
    /// In metres.
    pub(crate) accuracy: f64,
}

impl CdpPage {
    /// Has the page show `values` in place of its machine's, from now on,
    /// within a deadline of opening a page.
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave some of the values set.
    pub(crate) async fn emulate_opening(&self, values: &Emulated) -> Result<(), CallError> {
        if let Some(zone) = &values.time_zone {
            self.page
                .opening_command(&emulation::SetTimezoneOverrideParams {
                    timezone_id: zone.clone(),
                })
                .await?;
        }
        if let Some(locale) = &values.locale {
            self.page
                .opening_command(&emulation::SetLocaleOverrideParams {
                    locale: Some(locale.clone()),
                })
                .await?;
        }
        if let Some(position) = values.geolocation {
            self.page.opening_command(&geolocation(position)).await?;
        }
        if let Some(cores) = values.core_count {
            self.page
                .opening_command(&emulation::SetHardwareConcurrencyOverrideParams {
                    hardware_concurrency: i64::from(cores),
                })
                .await?;
        }
        Ok(())
    }

    /// Adds `source` to the scripts run in the main world of every document
    /// the page loads from now on, and of the one it holds now, before
    /// their own scripts, within a deadline of opening a page.
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses.
    pub(crate) async fn add_init_script_opening(&self, source: String) -> Result<(), CallError> {
        self.page
            .opening_command(&page::AddScriptToEvaluateOnNewDocumentParams {
                source,
                world_name: None,
                include_command_line_api: None,
                run_immediately: Some(true),
            })
            .await
            .map(drop)
    }
}

/// The override that has a page's geolocation answer `position`.
const fn geolocation(position: Geolocation) -> emulation::SetGeolocationOverrideParams {
    emulation::SetGeolocationOverrideParams {
        latitude: Some(position.latitude),
        longitude: Some(position.longitude),
        accuracy: Some(position.accuracy),
        altitude: None,
        altitude_accuracy: None,
        heading: None,
        speed: None,
    }
}
