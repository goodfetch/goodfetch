//! The Kennel's pure model (ADR 0141): connection changes, the settings and
//! the Daemon start's outcomes go in; the view the Tray draws and the
//! effects to carry out come out. It knows no toolkit, socket or clock, so
//! its tests script the inputs and read the view.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::daemon::{DaemonHealth, Unready};

use crate::client::{ConnectionChange, DaemonIdentity, Refusal};
use crate::daemon_start::{InstalledProduct, StartFailure};
use crate::settings::{Settings, SettingsError};

#[cfg(test)]
mod tests;

/// What reaches the State.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// The Client's Connection to the Daemon changed.
    Connection(ConnectionChange),
    /// The installed Product found at launch, whose `daemon start` the
    /// Kennel offers; none when no Product is installed.
    Installed(Option<InstalledProduct>),
    /// The Kennel's settings, read at launch and on each change of their
    /// file.
    Settings(Result<Settings, SettingsError>),
    /// The `daemon start` the person chose has ended.
    StartEnded(Result<(), StartFailure>),
}

/// What a person can choose in the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Choice {
    /// Run the installed Product's `daemon start`.
    StartDaemon,
    /// Quit the Kennel, which stops none of the Runs.
    Quit,
}

/// What the Kennel carries out for the State.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Run this Product's `daemon start`, then look for the Daemon again.
    StartDaemon(InstalledProduct),
    /// Quit.
    Quit,
}

/// The one verdict the icon shows, worst first (ADR 0141); later tickets
/// add the states between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// No Daemon this Kennel can talk to.
    Unreachable,
    /// A Daemon is reached and nothing needs the person.
    Idle,
}

/// One row of the menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A line the person reads.
    Text(String),
    /// A line the person chooses.
    Choice {
        /// What the line says.
        label: String,
        /// What choosing it does.
        choice: Choice,
    },
    /// A separator.
    Separator,
}

/// What the Tray draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    /// The icon's verdict.
    pub verdict: Verdict,
    /// The menu, top to bottom.
    pub rows: Vec<Row>,
}

/// The Daemon as the Client last reported it.
#[derive(Clone, Debug, PartialEq)]
enum Daemon {
    /// No report yet.
    Looking,
    /// None runs.
    None,
    /// Reached, with its last health reading on this Connection.
    Reached(DaemonIdentity, Option<DaemonHealth>),
    /// One runs, and refused this Kennel.
    Refused(Refusal),
}

/// The Daemon start the person chose.
#[derive(Clone, Debug, PartialEq)]
enum Start {
    /// None chosen, or the last one is over.
    Idle,
    /// `daemon start` runs.
    Running,
    /// `daemon start` ended well, and the Client looks for the Daemon
    /// again.
    Started,
    /// The last one failed.
    Failed(StartFailure),
}

/// The Kennel's model.
#[derive(Debug)]
pub struct State {
    daemon: Daemon,
    start: Start,
    installed: Option<InstalledProduct>,
    settings: Settings,
    settings_error: Option<SettingsError>,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    /// The State at launch: no report from the Client yet, no installed
    /// Product found and the default settings.
    pub fn new() -> Self {
        Self {
            daemon: Daemon::Looking,
            start: Start::Idle,
            installed: None,
            settings: Settings::default(),
            settings_error: None,
        }
    }

    /// Takes `input` in, and gives the effects it calls for: none so far,
    /// since the Kennel never starts a Daemon by itself.
    pub fn apply(&mut self, input: Input) -> Vec<Effect> {
        match input {
            Input::Connection(change) => {
                self.daemon = match change {
                    ConnectionChange::NoDaemon => Daemon::None,
                    // Each Connection starts with no reading: nothing the
                    // last one showed carries over.
                    ConnectionChange::Connected(identity) => Daemon::Reached(identity, None),
                    ConnectionChange::Refused(refusal) => Daemon::Refused(refusal),
                    ConnectionChange::Health(reading) => {
                        // A reading the Client sent before it lost the
                        // Daemon is of no Daemon now.
                        if let Daemon::Reached(_, health) = &mut self.daemon {
                            *health = Some(reading);
                        }
                        return Vec::new();
                    }
                };
                // A start that ended well waits for the Client's next
                // report; a reached Daemon makes a failed one moot.
                let reached = matches!(self.daemon, Daemon::Reached(..));
                if self.start == Start::Started
                    || (reached && matches!(self.start, Start::Failed(_)))
                {
                    self.start = Start::Idle;
                }
            }
            Input::Installed(installed) => self.installed = installed,
            Input::Settings(Ok(settings)) => {
                self.settings = settings;
                self.settings_error = None;
            }
            Input::Settings(Err(error)) => {
                self.settings = Settings::default();
                self.settings_error = Some(error);
            }
            Input::StartEnded(Ok(())) => self.start = Start::Started,
            Input::StartEnded(Err(failure)) => self.start = Start::Failed(failure),
        }
        Vec::new()
    }

    /// The person chose `choice` in the menu; a choice the menu no longer
    /// offers does nothing.
    pub fn choose(&mut self, choice: Choice) -> Vec<Effect> {
        match choice {
            Choice::Quit => vec![Effect::Quit],
            Choice::StartDaemon => match self.startable() {
                Some(installed) => {
                    let effect = Effect::StartDaemon(installed.clone());
                    self.start = Start::Running;
                    vec![effect]
                }
                None => Vec::new(),
            },
        }
    }

    /// What the Tray draws now.
    pub fn view(&self) -> View {
        let verdict = match self.daemon {
            Daemon::Reached(..) => Verdict::Idle,
            Daemon::Looking | Daemon::None | Daemon::Refused(_) => Verdict::Unreachable,
        };
        let mut rows = Vec::new();
        match (&self.start, &self.daemon) {
            (Start::Running | Start::Started, _) => {
                rows.push(Row::Text("Starting the Daemon…".to_owned()));
            }
            (_, Daemon::Looking) => rows.push(Row::Text("Looking for the Daemon…".to_owned())),
            (_, Daemon::Reached(identity, health)) => {
                rows.push(Row::Text(format!(
                    "{} Daemon {}",
                    product_name(identity.product),
                    identity.product_version
                )));
                if let Some(health) = health {
                    rows.push(Row::Text(health_text(health)));
                }
            }
            (_, Daemon::Refused(refusal)) => rows.push(Row::Text(refused(refusal))),
            (start, Daemon::None) => {
                rows.push(Row::Text("No Daemon".to_owned()));
                if let Start::Failed(failure) = start {
                    rows.push(Row::Text(format!("The Daemon did not start: {failure}")));
                }
                if self.settings.controls {
                    rows.push(match self.installed {
                        Some(_) => Row::Choice {
                            label: "Start Daemon".to_owned(),
                            choice: Choice::StartDaemon,
                        },
                        None => Row::Text(
                            "Start Daemon: no installed Collie or Foxhound was found".to_owned(),
                        ),
                    });
                }
            }
        }
        if let Some(error) = &self.settings_error {
            rows.push(Row::Text(format!(
                "kennel.toml cannot be read, so the defaults apply: {error}"
            )));
        }
        rows.push(Row::Separator);
        rows.push(Row::Choice {
            label: "Quit".to_owned(),
            choice: Choice::Quit,
        });
        View { verdict, rows }
    }

    /// The Product whose `daemon start` the menu offers now.
    fn startable(&self) -> Option<&InstalledProduct> {
        let idle = matches!(self.start, Start::Idle | Start::Failed(_));
        let offered = idle && self.daemon == Daemon::None && self.settings.controls;
        self.installed.as_ref().filter(|_| offered)
    }
}

/// What the menu says of a Daemon that refused the Kennel.
fn refused(refusal: &Refusal) -> String {
    match refusal {
        Refusal::ProtocolVersion { engine, needs } => format!(
            "The Daemon speaks protocol {engine}, and this Kennel needs a Daemon of protocol {needs}"
        ),
        Refusal::HandOver => "The Daemon is handing over to its major's newer version".to_owned(),
        Refusal::Other { code } => format!("The Daemon refused this Kennel: {code}"),
    }
}

/// What the menu says of the reached Daemon's health: serving, draining,
/// or why it takes no new Run.
fn health_text(health: &DaemonHealth) -> String {
    if !health.live {
        return "Not live: its Runs' registry does not respond".to_owned();
    }
    if health.ready {
        return "Serving".to_owned();
    }
    let draining = health.unready.contains(&Unready::Draining);
    let others: Vec<&str> = health
        .unready
        .iter()
        .filter_map(|unready| match unready {
            Unready::Draining => None,
            Unready::RunStateNotWritable => Some("its run state cannot be written"),
            Unready::BrowsersMissing => Some("a browser it uses is not installed"),
            Unready::GovernorLastResort => Some("its Resource governor is at its last resort"),
            Unready::StorageCritical => Some("its storage is at the critical level"),
        })
        .collect();
    match (draining, others.is_empty()) {
        (true, true) => "Draining: it takes no new Run".to_owned(),
        (true, false) => format!("Draining: it takes no new Run; {}", others.join("; ")),
        (false, true) => "Takes no new Run".to_owned(),
        (false, false) => format!("Takes no new Run: {}", others.join("; ")),
    }
}

fn product_name(product: Product) -> &'static str {
    match product {
        Product::Collie => "Collie",
        Product::Foxhound => "Foxhound",
    }
}
