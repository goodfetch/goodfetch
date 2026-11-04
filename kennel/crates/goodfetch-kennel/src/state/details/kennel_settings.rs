//! The details window's Kennel settings view: each of the Kennel's own
//! settings as `kennel.toml` holds them, a switch for each and the
//! coalescing period, grouped as the README lists them, with why the file
//! cannot be read or why an edit failed.

use crate::settings::{Settings, Switch};
use crate::text;

/// The most characters of a label or a note.
const LINE_MOST: usize = 300;

/// What the Kennel settings view shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KennelSettingsView {
    /// Why the file cannot be read, and the defaults apply, or why the
    /// last edit failed.
    pub note: Option<String>,
    /// Where the settings are kept.
    pub about: String,
    /// The switches, by section.
    pub sections: Vec<SettingsSection>,
}

/// A titled group of switches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsSection {
    /// What the switches cover.
    pub title: String,
    /// The switches, in order.
    pub switches: Vec<SwitchLine>,
    /// The coalescing period, in the desktop notices' section.
    pub period: Option<PeriodLine>,
}

/// The minutes over which a burst of desktop notices coalesces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeriodLine {
    /// What the minutes are.
    pub label: String,
    /// The minutes; 0 sends each notice at once.
    pub minutes: u32,
}

/// A switch as the view shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchLine {
    /// The switch, which an edit names.
    pub switch: Switch,
    /// What it turns on.
    pub label: String,
    /// Whether it is on.
    pub on: bool,
}

/// The sections, each with its title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Kennel,
    Notices,
    Views,
}

impl Section {
    const ALL: [Self; 3] = [Self::Kennel, Self::Notices, Self::Views];

    fn title(self) -> &'static str {
        match self {
            Self::Kennel => "The Kennel",
            Self::Notices => "Desktop notices",
            Self::Views => "The window's views",
        }
    }
}

/// The section `switch` is shown in, and its label.
fn place(switch: Switch) -> (Section, &'static str) {
    match switch {
        Switch::Controls => (Section::Kennel, "Controls: off, a read-only Kennel"),
        Switch::ConfirmStop => (Section::Kennel, "Ask before Stop"),
        Switch::Window => (Section::Kennel, "The details window: off, a menu only"),
        Switch::KeepDaemonAlive => (
            Section::Kennel,
            "Keep the Daemon alive while the Kennel is connected",
        ),
        Switch::StartAtLogin => (Section::Kennel, "Start at login (macOS)"),
        Switch::HiddenIconCheck => (
            Section::Kennel,
            "Check at launch that the icon shows in the menu bar (macOS)",
        ),
        Switch::Notices => (Section::Notices, "Desktop notices"),
        Switch::NeedsYouNotice => (Section::Notices, "A new Needs you item"),
        Switch::RunFailedNotice => (Section::Notices, "A Run failed"),
        Switch::RunStoppedByLimitNotice => (Section::Notices, "A Run stopped by a Limit"),
        Switch::DaemonLostNotice => (Section::Notices, "The Daemon lost while a Run was live"),
        Switch::RunFinishedNotice => (Section::Notices, "A Run that ended well"),
        Switch::RunsView => (
            Section::Views,
            "The Runs table: off, no Run is listed or chosen",
        ),
        Switch::RunProgressView => (Section::Views, "A Run's progress"),
        Switch::RunUsageView => (Section::Views, "A Run's Usage"),
        Switch::RunFailuresView => (Section::Views, "A Run's failures by Error code (Foxhound)"),
        Switch::RunReportView => (Section::Views, "A Run's report"),
        Switch::RecordsView => (Section::Views, "Records (Foxhound)"),
        Switch::DaemonSettingsView => (Section::Views, "Daemon settings"),
        Switch::GovernorView => (Section::Views, "Resource governor"),
        Switch::KennelSettingsView => (Section::Views, "Kennel settings"),
    }
}

/// The view of `settings`, with `note` above them.
pub(super) fn view(settings: &Settings, note: Option<&str>) -> KennelSettingsView {
    let sections = Section::ALL
        .into_iter()
        .map(|section| SettingsSection {
            title: text::shown(section.title(), LINE_MOST),
            switches: Switch::ALL
                .into_iter()
                .filter_map(|switch| {
                    let (shown_in, label) = place(switch);
                    (shown_in == section).then(|| SwitchLine {
                        switch,
                        label: text::shown(label, LINE_MOST),
                        on: switch.of(settings),
                    })
                })
                .collect(),
            period: (section == Section::Notices).then(|| PeriodLine {
                label: text::shown(
                    "A burst coalesces over, in minutes (0: each at once)",
                    LINE_MOST,
                ),
                minutes: settings.desktop_notices.period_minutes,
            }),
        })
        .collect();
    KennelSettingsView {
        note: note.map(|note| text::shown(note, LINE_MOST)),
        about: text::shown(
            "Kept in kennel.toml, in the Goodfetch home: a change made in the file shows here.",
            LINE_MOST,
        ),
        sections,
    }
}
