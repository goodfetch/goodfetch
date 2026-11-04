//! The desktop notices (ADR 0141): what each transition that needs a
//! person says, and how a burst coalesces.
//!
//! - **Which.** The State hands over a notice for each transition whose
//!   type is on: a new Needs you item, a Run failed, a Run stopped by a
//!   Limit, the Daemon lost while a Run was live, and, off by default, a
//!   Run that ended well. Nothing is kept before the settings are read, or
//!   while `notifications` is off.
//! - **Coalescing.** The first notice goes out at once. Those that come
//!   within the period after it are held, and one summary of them goes
//!   out when the period ends, which starts a new period. A period with
//!   nothing held ends the burst, and the next notice goes out at once. A
//!   summary of one notice is that notice. A period of 0 sends each at
//!   once.
//! - **Text.** A notice's title and body pass through the Text function.
//!
//! The instants come from the caller: this module reads no clock.

use std::time::Instant;

use goodfetch_protocol::catalog::Product;

use crate::settings::DesktopNotices;
use crate::text;

/// The most characters of a notice's title.
const TITLE_MOST: usize = 60;

/// The most characters of a notice's body.
const BODY_MOST: usize = 200;

/// A desktop notice to send, its text made safe to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// Its first line.
    pub title: String,
    /// What it says under the title.
    pub body: String,
}

impl Notice {
    fn new(title: &str, body: &str) -> Self {
        Self {
            title: text::shown(title, TITLE_MOST),
            body: text::shown(body, BODY_MOST),
        }
    }
}

/// The notices to send now, and when held ones fall due.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notices {
    /// Send these now, in order.
    pub send: Vec<Notice>,
    /// The summary of the held notices falls due at this instant; none
    /// when nothing is held.
    pub due: Option<Instant>,
}

/// A type of desktop notice, each a setting of its own, in the order a
/// summary names them: worst first, as the verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Kind {
    DaemonLost,
    /// The Kennel's icon is hidden, with no window to say so: on whenever
    /// desktop notices are.
    IconHidden,
    NeedsYou,
    RunFailed,
    RunStoppedByLimit,
    RunFinished,
}

impl Kind {
    /// Whether `notifications` turns this type on.
    fn on(self, notices: &DesktopNotices) -> bool {
        match self {
            Self::DaemonLost => notices.daemon_lost,
            Self::IconHidden => true,
            Self::NeedsYou => notices.needs_you,
            Self::RunFailed => notices.run_failed,
            Self::RunStoppedByLimit => notices.run_stopped_by_limit,
            Self::RunFinished => notices.run_finished,
        }
    }

    /// What a summary says of `count` notices of this type.
    fn counted(self, count: usize) -> String {
        match (self, count) {
            (Self::DaemonLost, 1) => "the Daemon lost while a Run was live".to_owned(),
            (Self::DaemonLost, _) => format!("the Daemon lost {count} times while Runs were live"),
            (Self::IconHidden, _) => "the Kennel's icon hidden in the menu bar".to_owned(),
            (Self::NeedsYou, 1) => "1 new Needs you item".to_owned(),
            (Self::NeedsYou, _) => format!("{count} new Needs you items"),
            (Self::RunFailed, 1) => "1 Run failed".to_owned(),
            (Self::RunFailed, _) => format!("{count} Runs failed"),
            (Self::RunStoppedByLimit, 1) => "1 Run stopped by a Limit".to_owned(),
            (Self::RunStoppedByLimit, _) => format!("{count} Runs stopped by a Limit"),
            (Self::RunFinished, 1) => "1 Run finished".to_owned(),
            (Self::RunFinished, _) => format!("{count} Runs finished"),
        }
    }
}

/// A notice of a type, before coalescing.
#[derive(Clone, Debug)]
struct Pending {
    kind: Kind,
    notice: Notice,
}

/// The notice of a new Needs you item, which says what waits.
pub(super) fn needs_you(summary: &str) -> Notice {
    Notice::new("Needs you", summary)
}

/// The notice of the Kennel's hidden icon, asking the person to free
/// space in the menu bar, when no window can say so.
pub(super) fn icon_hidden() -> Notice {
    Notice::new(
        "The Kennel's icon is hidden",
        "Free space in the menu bar, by quitting or removing other menu bar items, and the icon shows again.",
    )
}

/// How a Run ended, for its notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ended {
    Failed,
    StoppedByLimit,
    Finished,
}

impl Ended {
    /// The type of its notice.
    pub(super) fn kind(self) -> Kind {
        match self {
            Self::Failed => Kind::RunFailed,
            Self::StoppedByLimit => Kind::RunStoppedByLimit,
            Self::Finished => Kind::RunFinished,
        }
    }
}

/// The notice of the Run named `name` that ended as `ended` says.
pub(super) fn run_ended(ended: Ended, name: &str) -> Notice {
    let title = match ended {
        Ended::Failed => "Run failed",
        Ended::StoppedByLimit => "Run stopped by a Limit",
        Ended::Finished => "Run finished",
    };
    Notice::new(title, name)
}

/// The notice of a Daemon of `product` lost while `live` Runs were live.
pub(super) fn daemon_lost(product: Product, live: usize) -> Notice {
    let product = match product {
        Product::Collie => "Collie",
        Product::Foxhound => "Foxhound",
    };
    let runs = if live == 1 {
        "1 Run was".to_owned()
    } else {
        format!("{live} Runs were")
    };
    Notice::new(
        "Daemon lost",
        &format!("The {product} Daemon is gone, and {runs} live on it"),
    )
}

/// The notice that sums `held` up: the one notice itself, or how many of
/// each type, worst first.
fn summary(mut held: Vec<Pending>) -> Notice {
    if held.len() == 1 {
        return held.remove(0).notice;
    }
    let mut kinds: Vec<Kind> = held.iter().map(|pending| pending.kind).collect();
    kinds.sort_unstable();
    let mut counts: Vec<(Kind, usize)> = Vec::new();
    for kind in kinds {
        match counts.last_mut() {
            Some((last, count)) if *last == kind => *count += 1,
            _ => counts.push((kind, 1)),
        }
    }
    let body: Vec<String> = counts
        .into_iter()
        .map(|(kind, count)| kind.counted(count))
        .collect();
    Notice::new(&format!("{} more changes", held.len()), &body.join("; "))
}

/// The notices decided and not yet sent, and when the last went out.
#[derive(Debug, Default)]
pub(super) struct Coalescer {
    /// The settings' notices, once read.
    settings: Option<DesktopNotices>,
    /// Decided since the last [`Coalescer::take`].
    fresh: Vec<Pending>,
    /// Held for the summary at the period's end.
    held: Vec<Pending>,
    /// When the last notice or summary went out.
    sent_at: Option<Instant>,
}

impl Coalescer {
    /// The settings read now: off drops every notice not sent and ends
    /// the burst; a type turned off drops its own.
    pub(super) fn settings(&mut self, notices: DesktopNotices) {
        if notices.enabled {
            self.fresh.retain(|pending| pending.kind.on(&notices));
            self.held.retain(|pending| pending.kind.on(&notices));
        } else {
            self.fresh.clear();
            self.held.clear();
            self.sent_at = None;
        }
        self.settings = Some(notices);
    }

    /// Whether desktop notices are on: the settings are read and say so.
    pub(super) fn notifying(&self) -> bool {
        self.settings.is_some_and(|notices| notices.enabled)
    }

    /// Whether a notice of `kind` would be sent.
    pub(super) fn wants(&self, kind: Kind) -> bool {
        self.settings
            .is_some_and(|notices| notices.enabled && kind.on(&notices))
    }

    /// Adds the notice `make` gives, when its `kind` is wanted.
    pub(super) fn push(&mut self, kind: Kind, make: impl FnOnce() -> Notice) {
        if self.wants(kind) {
            self.fresh.push(Pending {
                kind,
                notice: make(),
            });
        }
    }

    /// The notices to send at `now`, and when the held ones fall due.
    pub(super) fn take(&mut self, now: Instant) -> Notices {
        let fresh = std::mem::take(&mut self.fresh);
        let Some(notices) = self.settings.filter(|notices| notices.enabled) else {
            return Notices {
                send: Vec::new(),
                due: None,
            };
        };
        let period = notices.period();
        let mut send = Vec::new();
        if period.is_zero() {
            send.extend(
                self.held
                    .drain(..)
                    .chain(fresh)
                    .map(|pending| pending.notice),
            );
            return Notices { send, due: None };
        }
        let open = self.sent_at.is_some_and(|at| now < at + period);
        if open {
            self.held.extend(fresh);
        } else if self.held.is_empty() {
            let mut fresh = fresh.into_iter();
            if let Some(first) = fresh.next() {
                send.push(first.notice);
                self.sent_at = Some(now);
            }
            self.held.extend(fresh);
        } else {
            self.held.extend(fresh);
            send.push(summary(std::mem::take(&mut self.held)));
            self.sent_at = Some(now);
        }
        let due = self
            .sent_at
            .filter(|_| !self.held.is_empty())
            .map(|at| at + period);
        Notices { send, due }
    }
}
