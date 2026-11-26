//! The primitive that has a page show other values than its machine's
//! through the browser's own overrides (ADR 0035's `basic` Fingerprint
//! method): its user agent and client hints, its screen, its time zone,
//! its locale, its geolocation and its core count; and the one that has it
//! run a script in each document's main world before the document's own,
//! from its opening (`js_injection`).

use goodfetch_cdp::generated::{emulation, page};
use goodfetch_engine::CallError;
use goodfetch_identity_profile::{BrandVersion, ClientHints};

use super::CdpPage;

/// The values a page shows in place of its machine's; each one left out
/// stays the machine's.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Emulated {
    /// The user agent its requests send and `navigator` reads.
    pub(crate) user_agent: Option<UserAgentOverride>,
    /// The screen size `screen` reads; the page's viewport and pixel ratio
    /// stay its window's.
    pub(crate) screen: Option<ScreenSize>,
    /// An IANA time zone, such as `Europe/Paris`.
    pub(crate) time_zone: Option<String>,
    /// A BCP 47 locale, such as `fr-FR`.
    pub(crate) locale: Option<String>,
    /// Where the page's geolocation says it is.
    pub(crate) geolocation: Option<Geolocation>,
    /// The core count `navigator.hardwareConcurrency` reads.
    pub(crate) core_count: Option<u32>,
}

/// A user agent a page shows, with what goes with it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UserAgentOverride {
    /// The `User-Agent` header and `navigator.userAgent`.
    pub(crate) user_agent: String,
    /// The `Accept-Language` header, such as `fr-FR,fr;q=0.9`.
    pub(crate) accept_language: Option<String>,
    /// `navigator.platform`, such as `Win32`.
    pub(crate) platform: String,
    /// The client hints, which a browser without them leaves out.
    pub(crate) client_hints: Option<ClientHints>,
}

/// The size of the screen a page reports, in CSS pixels, and the part of
/// it a window may take. `basic` shows the whole screen as available:
/// `Emulation.setDeviceMetricsOverride` takes no available size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScreenSize {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) avail_width: u32,
    pub(crate) avail_height: u32,
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
        if let Some(agent) = &values.user_agent {
            self.page
                .opening_command(&emulation::SetUserAgentOverrideParams {
                    user_agent: agent.user_agent.clone(),
                    accept_language: agent.accept_language.clone(),
                    platform: Some(agent.platform.clone()),
                    user_agent_metadata: agent.client_hints.as_ref().map(metadata),
                })
                .await?;
        }
        if let Some(screen) = values.screen {
            self.page.opening_command(&screen_size(screen)).await?;
        }
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

/// The client hints `navigator.userAgentData` and the `Sec-CH-UA` headers
/// show, as `Emulation.setUserAgentOverride` takes them.
fn metadata(hints: &ClientHints) -> emulation::UserAgentMetadata {
    let brands = |list: &[BrandVersion]| {
        list.iter()
            .map(|brand| emulation::UserAgentBrandVersion {
                brand: brand.brand.clone(),
                version: brand.version.clone(),
            })
            .collect()
    };
    emulation::UserAgentMetadata {
        brands: Some(brands(&hints.brands)),
        full_version_list: Some(brands(&hints.full_version_list)),
        full_version: Some(hints.ua_full_version.clone()),
        platform: hints.platform.clone(),
        platform_version: hints.platform_version.clone(),
        architecture: hints.architecture.clone(),
        model: hints.model.clone(),
        mobile: hints.mobile,
        bitness: Some(hints.bitness.clone()),
        wow64: Some(false),
        form_factors: None,
    }
}

/// The override that has a page's screen report `screen`: a width, height
/// and scale factor of 0 leave the viewport and pixel ratio the window's.
fn screen_size(screen: ScreenSize) -> emulation::SetDeviceMetricsOverrideParams {
    emulation::SetDeviceMetricsOverrideParams {
        width: 0,
        height: 0,
        device_scale_factor: 0.0,
        mobile: false,
        scale: None,
        screen_width: Some(i64::from(screen.width)),
        screen_height: Some(i64::from(screen.height)),
        position_x: None,
        position_y: None,
        dont_set_visible_size: None,
        screen_orientation: None,
        viewport: None,
        display_feature: None,
        device_posture: None,
        scrollbar_type: None,
        screen_orientation_lock_emulation: None,
        viewport_meta: None,
        text_layout_mode: None,
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
