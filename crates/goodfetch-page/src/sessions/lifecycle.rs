//! A Session's and a page's transition tables (ADR 0128; spec #594): each
//! changes state only through [`session`] or [`page`], which match the
//! state, then the event inside each state's arm, with no wildcard, and
//! refuse a pair the table does not allow with an [`InvalidTransition`]
//! naming the state and the event. A refusal a program caused names the
//! user-facing Error code (`closed`, `page_crashed`, `session_reserved`);
//! one only a bug can cause is `internal`, logged at `error` with the
//! object, the state and the event. A late event, such as a disconnection
//! racing a close, is an allowed pair that changes nothing.
//!
//! A Session's table. A cell names the next state and its effect; *late*
//! changes nothing, *bug* is `internal`. The Session's source and owner are
//! fixed [`Attributes`] that decide some cells (spec #594's decision 9).
//!
//! | Event, in the state: | `Opening` | `Open` | `Lent` | `Offline` | `Closing` | `Closed` | `Crashed` |
//! |---|---|---|---|---|---|---|---|
//! | `SourceReady` | `Open`, opened | bug | bug | late | late | late | bug |
//! | `OpenFailed` | `Closed`, the open fails | bug | bug | late | late | late | bug |
//! | `Call` | `Opening`, waits | `Open`, served | `Lent`, served | `Offline`, waits | `closed` | `closed` | `page_crashed` |
//! | `Lend` | bug | `Lent`, lent | `session_reserved` | `Offline`, waits | `closed` | `closed` | `page_crashed` |
//! | `AttachEnded` | bug | bug | `Open`, attach ended | bug | late | late | late |
//! | `Close` | `Closing`, open abandoned | `Closing`, children close | `Closing`, attach ended and children close | `Offline`, waits | late | late | `Crashed`, released |
//! | `ChildrenClosed` | bug | bug | bug | bug | `Closed`, released | bug | bug |
//! | `BrowserQuit` | `Closed`, the open fails | `Closed`, quit | `Closed`, quit | bug | late | late | late |
//! | `BrowserCrash` | `Closed`, the open fails | `Crashed`, crash | `Crashed`, crash | bug | late | late | late |
//! | `HostOffline` | person's host: `Offline`, suspended; hosted: `Closed`, the open fails | person's host: `Offline`, suspended; hosted: `Open`, unreachable | person's host: `Offline`, attach ended and suspended; hosted: `Lent`, unreachable | bug | late | late | late |
//! | `HostBack` | late | late | late | `Open`, resumed, or `Opening` again; `Closed`, quit, after a quit | late | late | late |
//! | `Retire` | bug | a pool's Session: `Closing`, children close | bug | bug | late | late | late |
//!
//! `HostOffline` on a Session no Browser host serves, and `Retire` on one
//! the pool did not open, or on a Borrowed browser or a Profile directory
//! used in place, are bugs. A crash in `Closing` is late and does not count
//! in Usage (decision 14). `close()` on a `Crashed` Session releases what
//! is left and it stays `Crashed` (decision 3). An abandoned open that
//! finishes once its close ended is late in `Closed`, as the catalogue's
//! `L` has it; no open is pending in `Crashed`, which `Opening` never
//! reaches.
//!
//! A page's table, its states and events those of spec #594's part 2.
//!
//! | Event, in the state: | `Open` | `Navigating` | `Paused` | `Crashed` | `Closing` | `Closed` |
//! |---|---|---|---|---|---|---|
//! | `Gesture` | `Open`, served | `Navigating`, waits | `Paused`, waits | `page_crashed` | `closed` | `closed` |
//! | `Read` | `Open`, served | `Navigating`, served | `Paused`, served | `page_crashed` | `closed` | `closed` |
//! | `Goto` | `Navigating`, served | `Navigating`, interrupts the pending one | `Paused`, waits | `page_crashed` | `closed` | `closed` |
//! | `ExternalNav` | `Open` | `Open`, interrupted | `Paused` | late | late | late |
//! | `Committed` | bug | `Open`, committed | bug | late | late | late |
//! | `NavFailed` | bug | `Open`, the navigation fails | bug | late | late | late |
//! | `Judged` | `Open`, judged | bug | `Paused`, deferred | late | late | late |
//! | `Pause` | `Paused`, paused | `Navigating`, waits | `Paused`, waits | `page_crashed` | `closed` | `closed` |
//! | `PauseEnded` | bug | bug | `Open`, pause ended | late | late | late |
//! | `Close` | `Closing`, closes | `Closing`, closes | `Closing`, closes | `Crashed`, released | late | late |
//! | `TargetDestroyed` | `Closed`, closed in the browser | `Closed`, closed in the browser | `Closed`, closed in the browser | late | `Closed`, confirmed | late |
//! | `RendererCrash` | `Crashed`, renderer crash | `Crashed`, renderer crash | `Crashed`, renderer crash | late | late | late |
//! | `BrowserQuit` | `Closed`, quit | `Closed`, quit | `Closed`, quit | late | `Closed`, close overtaken | late |
//! | `BrowserCrash` | `Crashed`, browser crash | `Crashed`, browser crash | `Crashed`, browser crash | late | `Closed`, close overtaken | late |
//! | `TargetEvent` | `Open` | `Navigating` | `Paused` | late | late | late |
//! | `LeaseEnded` | `Closing`, closes | `Closing`, closes | `Closing`, closes | `Crashed`, released | late | late |
//! | `MemoryExceeded` | `Closing`, closes | `Closing`, closes | `Closing`, closes | `Crashed`, released | late | late |
//! | `EventsLost` | `Closing`, closes | `Closing`, closes | `Closing`, closes | `Crashed`, released | late | late |
//!
//! `EventsLost` is ADR 0146's: a Page whose reader fell 4,096 events behind
//! is closed, and its calls raise `page_events_lost`.
//!
//! A quit or a crash racing a close (`Closing`) ends the page with nothing
//! raised, since its calls already ended, and the crash does not count in
//! Usage (decision 14); the page leaves as a confirmed close does (P30). A
//! second navigation call on a `Navigating` page interrupts the pending
//! one, which raises `navigation_interrupted` (P37, decision 15).

#![deny(clippy::wildcard_enum_match_arm)]

use goodfetch_engine::transition::{InvalidTransition, Refusal};
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::page::PAGE_CRASHED;

/// Area 3's code for a second `page.lend()` on a Session already lent
/// (ADR 0089), a program's refusal.
const SESSION_RESERVED: &str = "session_reserved";

/// Where a Session is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionState {
    /// Its source is read, its browser launched or probed.
    Opening,
    /// It serves calls; its pages open and close under it.
    Open,
    /// A library is attached to one of its pages' endpoint.
    Lent,
    /// A person's own Browser host serving it lost its link; it takes no
    /// work until the link returns, and goes back to where it was.
    Offline(Suspended),
    /// `close()` was called: its pages and context close first.
    Closing,
    /// It ended and never reopens.
    Closed,
    /// Its browser process exited abnormally; terminal.
    Crashed,
}

/// Where an `Offline` Session goes back to when its link returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Suspended {
    /// Its open, run again on the returned link.
    Opening,
    /// Serving.
    Open,
}

/// What happens to a Session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "lending, Browser hosts and the Session pool wire their events with their tickets (spec #594)"
    )
)]
pub(crate) enum SessionEvent {
    /// The opening call finished.
    SourceReady,
    /// The opening call failed.
    OpenFailed,
    /// A program call the Session serves, such as `newPage`.
    Call,
    /// `page.lend()` on one of its pages.
    Lend,
    /// The attach of a lent page ended.
    AttachEnded,
    /// `session.close()`, or the Run or Connection closing it.
    Close,
    /// Its pages and context are closed.
    ChildrenClosed,
    /// Its browser quit, or a Borrowed browser's connection dropped with no
    /// quit sequence.
    BrowserQuit,
    /// Its launched browser exited non-zero or by a signal.
    BrowserCrash,
    /// The link of the Browser host serving it was lost.
    HostOffline,
    /// That link returned; `browser_quit` when it reports that the browser
    /// quit while it was down.
    HostBack {
        /// Whether the browser quit while the link was down.
        browser_quit: bool,
    },
    /// The Session pool retires it, on a Block cause.
    Retire,
}

/// What a Session is opened from, a fixed attribute that decides some
/// cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "each Session source kind joins with the ticket that reads it (ADR 0056)"
    )
)]
pub(crate) enum Source {
    /// A browser the engine launches, fresh.
    Launched,
    /// A Profile directory, copied.
    ProfileDirectory,
    /// A Profile directory used in place.
    InPlaceProfileDirectory,
    /// A Borrowed browser: `running_browser`, `cdp_url` or
    /// `client_browser`.
    Borrowed,
    /// A person's own Browser host.
    PersonsBrowserHost,
    /// A Browser host an operator runs.
    HostedBrowserHost,
}

impl Source {
    /// Whether the Session's browser is one this engine launches on this
    /// machine, whose connections the egress relay binds to an address of
    /// this machine (ADR 0044). A Borrowed browser, a third-party CDP
    /// browser among them, and a Browser host's are not: an address of this
    /// machine is never claimed for them (ADRs 0044 and 0056).
    pub(crate) fn launched_here(self) -> bool {
        match self {
            Self::Launched | Self::ProfileDirectory | Self::InPlaceProfileDirectory => true,
            Self::Borrowed | Self::PersonsBrowserHost | Self::HostedBrowserHost => false,
        }
    }
}

/// Who opened a Session, a fixed attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "an Agent and a crawl's Session pool open Sessions once their tickets land (spec #594)"
    )
)]
pub(crate) enum Owner {
    /// A program.
    Program,
    /// An Agent.
    Agent,
    /// A crawl's Session pool.
    Pool,
}

/// A Session's fixed attributes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Attributes {
    pub(crate) source: Source,
    pub(crate) owner: Owner,
}

/// What a Session does on an allowed event, besides changing state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionEffect {
    /// Nothing more: a late event, or an idempotent `close()`.
    Nothing,
    /// The call is served.
    Serve,
    /// The call waits until the state changes, then is asked again.
    Wait,
    /// The calls waiting on the open run.
    Opened,
    /// The open fails: the opening call and every call waiting on it raise
    /// its code.
    FailOpen,
    /// The open is abandoned: the opening call and every call waiting on
    /// it raise `call_cancelled`, `by: "program"`, `closed: "session"`.
    AbandonOpen,
    /// Calls in flight on its pages raise `call_cancelled`, `by:
    /// "program"`, `closed: "session"`; its pages and context close.
    CloseChildren,
    /// The lent page's endpoint opens.
    Lend,
    /// The attach ends: the library's pages close, overrides are applied
    /// again and its token is revoked.
    EndAttach,
    /// The attach ends, then the children close.
    EndAttachAndClose,
    /// Calls in flight on its pages raise `call_cancelled`, `by:
    /// "browser"`, `closed: "browser"`; it ends, and a lent attach with it.
    Quit,
    /// Each page in flight raises `page_crashed` with how the browser
    /// exited and emits `crash`; the crash counts in Usage, and a lent
    /// attach ends with it.
    Crash,
    /// It takes no work: leases expire and the Run waits in a Human pause.
    Suspend,
    /// The attach ends, then it is suspended.
    EndAttachAndSuspend,
    /// The call fails with `browser_host_unreachable`.
    Unreachable,
    /// The Run resumes on the returned link.
    Resume,
    /// The open runs again on the returned link.
    Reopen,
    /// What is left is released: a launched browser's slot goes back to
    /// the governor.
    Release,
}

/// A transition a Session's table refuses.
pub(crate) type InvalidSessionTransition = InvalidTransition<SessionState, SessionEvent>;

/// What `event` does to a Session of `attributes` in `state`: its next
/// state and its effect, or why it is refused.
#[expect(
    clippy::too_many_lines,
    reason = "one table of seven states by twelve events, which ADR 0128 keeps in one function"
)]
pub(crate) fn session(
    state: SessionState,
    event: SessionEvent,
    attributes: Attributes,
) -> Result<(SessionState, SessionEffect), InvalidSessionTransition> {
    let refused = |refusal| {
        Err(InvalidTransition {
            state,
            event,
            refusal,
        })
    };
    let late = Ok((state, SessionEffect::Nothing));
    match state {
        SessionState::Opening => match event {
            SessionEvent::SourceReady => Ok((SessionState::Open, SessionEffect::Opened)),
            SessionEvent::OpenFailed | SessionEvent::BrowserQuit | SessionEvent::BrowserCrash => {
                Ok((SessionState::Closed, SessionEffect::FailOpen))
            }
            SessionEvent::Call => Ok((SessionState::Opening, SessionEffect::Wait)),
            SessionEvent::Close => Ok((SessionState::Closing, SessionEffect::AbandonOpen)),
            SessionEvent::HostOffline => match attributes.source {
                Source::PersonsBrowserHost => Ok((
                    SessionState::Offline(Suspended::Opening),
                    SessionEffect::Suspend,
                )),
                Source::HostedBrowserHost => Ok((SessionState::Closed, SessionEffect::FailOpen)),
                Source::Launched
                | Source::ProfileDirectory
                | Source::InPlaceProfileDirectory
                | Source::Borrowed => refused(Refusal::Internal),
            },
            SessionEvent::HostBack { .. } => late,
            SessionEvent::Lend
            | SessionEvent::AttachEnded
            | SessionEvent::ChildrenClosed
            | SessionEvent::Retire => refused(Refusal::Internal),
        },
        SessionState::Open => match event {
            SessionEvent::Call => Ok((SessionState::Open, SessionEffect::Serve)),
            SessionEvent::Lend => Ok((SessionState::Lent, SessionEffect::Lend)),
            SessionEvent::Close => Ok((SessionState::Closing, SessionEffect::CloseChildren)),
            SessionEvent::BrowserQuit => Ok((SessionState::Closed, SessionEffect::Quit)),
            SessionEvent::BrowserCrash => Ok((SessionState::Crashed, SessionEffect::Crash)),
            SessionEvent::HostOffline => match attributes.source {
                Source::PersonsBrowserHost => Ok((
                    SessionState::Offline(Suspended::Open),
                    SessionEffect::Suspend,
                )),
                Source::HostedBrowserHost => Ok((SessionState::Open, SessionEffect::Unreachable)),
                Source::Launched
                | Source::ProfileDirectory
                | Source::InPlaceProfileDirectory
                | Source::Borrowed => refused(Refusal::Internal),
            },
            SessionEvent::HostBack { .. } => late,
            SessionEvent::Retire => match attributes.owner {
                Owner::Pool => match attributes.source {
                    Source::Launched
                    | Source::ProfileDirectory
                    | Source::PersonsBrowserHost
                    | Source::HostedBrowserHost => {
                        Ok((SessionState::Closing, SessionEffect::CloseChildren))
                    }
                    Source::Borrowed | Source::InPlaceProfileDirectory => {
                        refused(Refusal::Internal)
                    }
                },
                Owner::Program | Owner::Agent => refused(Refusal::Internal),
            },
            SessionEvent::SourceReady
            | SessionEvent::OpenFailed
            | SessionEvent::AttachEnded
            | SessionEvent::ChildrenClosed => refused(Refusal::Internal),
        },
        SessionState::Lent => match event {
            SessionEvent::Call => Ok((SessionState::Lent, SessionEffect::Serve)),
            SessionEvent::Lend => refused(Refusal::Program(SESSION_RESERVED)),
            SessionEvent::AttachEnded => Ok((SessionState::Open, SessionEffect::EndAttach)),
            SessionEvent::Close => Ok((SessionState::Closing, SessionEffect::EndAttachAndClose)),
            SessionEvent::BrowserQuit => Ok((SessionState::Closed, SessionEffect::Quit)),
            SessionEvent::BrowserCrash => Ok((SessionState::Crashed, SessionEffect::Crash)),
            SessionEvent::HostOffline => match attributes.source {
                Source::PersonsBrowserHost => Ok((
                    SessionState::Offline(Suspended::Open),
                    SessionEffect::EndAttachAndSuspend,
                )),
                Source::HostedBrowserHost => Ok((SessionState::Lent, SessionEffect::Unreachable)),
                Source::Launched
                | Source::ProfileDirectory
                | Source::InPlaceProfileDirectory
                | Source::Borrowed => refused(Refusal::Internal),
            },
            SessionEvent::HostBack { .. } => late,
            SessionEvent::SourceReady
            | SessionEvent::OpenFailed
            | SessionEvent::ChildrenClosed
            | SessionEvent::Retire => refused(Refusal::Internal),
        },
        SessionState::Offline(suspended) => match event {
            SessionEvent::SourceReady | SessionEvent::OpenFailed => late,
            SessionEvent::Call | SessionEvent::Lend | SessionEvent::Close => {
                Ok((state, SessionEffect::Wait))
            }
            SessionEvent::HostBack { browser_quit: true } => {
                Ok((SessionState::Closed, SessionEffect::Quit))
            }
            SessionEvent::HostBack {
                browser_quit: false,
            } => match suspended {
                Suspended::Opening => Ok((SessionState::Opening, SessionEffect::Reopen)),
                Suspended::Open => Ok((SessionState::Open, SessionEffect::Resume)),
            },
            SessionEvent::AttachEnded
            | SessionEvent::ChildrenClosed
            | SessionEvent::BrowserQuit
            | SessionEvent::BrowserCrash
            | SessionEvent::HostOffline
            | SessionEvent::Retire => refused(Refusal::Internal),
        },
        SessionState::Closing => match event {
            SessionEvent::Call | SessionEvent::Lend => refused(Refusal::Program(CLOSED.name)),
            SessionEvent::ChildrenClosed => Ok((SessionState::Closed, SessionEffect::Release)),
            SessionEvent::SourceReady
            | SessionEvent::OpenFailed
            | SessionEvent::AttachEnded
            | SessionEvent::Close
            | SessionEvent::BrowserQuit
            | SessionEvent::BrowserCrash
            | SessionEvent::HostOffline
            | SessionEvent::HostBack { .. }
            | SessionEvent::Retire => late,
        },
        SessionState::Closed => match event {
            SessionEvent::Call | SessionEvent::Lend => refused(Refusal::Program(CLOSED.name)),
            SessionEvent::SourceReady
            | SessionEvent::OpenFailed
            | SessionEvent::AttachEnded
            | SessionEvent::Close
            | SessionEvent::BrowserQuit
            | SessionEvent::BrowserCrash
            | SessionEvent::HostOffline
            | SessionEvent::HostBack { .. }
            | SessionEvent::Retire => late,
            SessionEvent::ChildrenClosed => refused(Refusal::Internal),
        },
        SessionState::Crashed => match event {
            SessionEvent::Call | SessionEvent::Lend => refused(Refusal::Program(PAGE_CRASHED.name)),
            SessionEvent::Close => Ok((SessionState::Crashed, SessionEffect::Release)),
            SessionEvent::AttachEnded
            | SessionEvent::BrowserQuit
            | SessionEvent::BrowserCrash
            | SessionEvent::HostOffline
            | SessionEvent::HostBack { .. }
            | SessionEvent::Retire => late,
            SessionEvent::SourceReady | SessionEvent::OpenFailed | SessionEvent::ChildrenClosed => {
                refused(Refusal::Internal)
            }
        },
    }
}

/// Where a page is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PageState {
    /// Loaded or idle.
    Open,
    /// A navigation a Collie call started has not committed.
    Navigating,
    /// A Human pause holds it.
    Paused,
    /// Its renderer or its browser crashed; terminal.
    Crashed,
    /// Its close was sent, or its handler's lease ended; its target is not
    /// yet destroyed.
    Closing,
    /// It ended and never reopens.
    Closed,
}

/// What happens to a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the tickets that bring navigation, the Page check, the Human pause and leases wire their events (spec #594)"
    )
)]
pub(crate) enum PageEvent {
    /// A call that sends input.
    Gesture,
    /// A read, a wait, `evaluate`, or an answer to a route or dialog.
    Read,
    /// `goto`, `reload`, `goBack` or `goForward`.
    Goto,
    /// A main-frame navigation no Collie call started commits.
    ExternalNav,
    /// The pending Collie navigation's document commits.
    Committed,
    /// The pending navigation fails.
    NavFailed,
    /// A committed document is judged by the Page check and Block
    /// classification.
    Judged,
    /// `page.pause()`, or a pause the engine raises in a call.
    Pause,
    /// The pause ends.
    PauseEnded,
    /// `page.close()`, or the Session or Run closing it.
    Close,
    /// Its target is destroyed.
    TargetDestroyed,
    /// Its renderer crashed.
    RendererCrash,
    /// Its Session's browser quit.
    BrowserQuit,
    /// Its Session's launched browser exited abnormally.
    BrowserCrash,
    /// Any other event of its target.
    TargetEvent,
    /// Its crawl handler's lease ended.
    LeaseEnded,
    /// The Resource governor ends it past its memory cap while memory is
    /// critical (ADR 0040).
    MemoryExceeded,
    /// Its reader fell 4,096 events behind, so the engine closes it (ADR
    /// 0146).
    EventsLost,
}

/// What a page does on an allowed event, besides changing state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PageEffect {
    /// Nothing more: the event is routed outside the table, or late, or an
    /// idempotent `close()`.
    Nothing,
    /// The call is served.
    Serve,
    /// The call waits until the state changes, then is asked again.
    Wait,
    /// The pending navigation returns its document's response, then the
    /// document is judged.
    Commit,
    /// The pending navigation raises its failure.
    FailNavigation,
    /// The pending navigation raises `navigation_interrupted`.
    Interrupt,
    /// The Page check and Block classification act on the document.
    Judge,
    /// The judgement waits for the pause to end.
    Defer,
    /// Every enclosing clock stops; later gestures wait.
    Pause,
    /// The pause returns how it ended; a deferred judgement and a waiting
    /// pause then run.
    EndPause,
    /// Calls in flight raise `call_cancelled`, `by: "program"`, `closed:
    /// "page"`; its target closes.
    Close,
    /// Calls in flight raise `call_cancelled`, `by: "browser"`, `closed:
    /// "page"`, and it emits `close`; its Session and other pages stay.
    ClosedInBrowser,
    /// Calls in flight raise `call_cancelled`, `by: "browser"`, `closed:
    /// "browser"`.
    Quit,
    /// Calls in flight raise `page_crashed`, and it emits `crash`.
    RendererCrash,
    /// Calls in flight raise `page_crashed` with how the browser exited,
    /// and it emits `crash`.
    BrowserCrash,
    /// The engine's own close is confirmed: it emits `close`.
    Confirm,
    /// Its browser quit or crashed while it closed: nothing is raised,
    /// since its calls already ended, the crash does not count in Usage,
    /// and it leaves as a confirmed close does, emitting `close` (P30).
    CloseOvertaken,
    /// What is left is released.
    Release,
}

/// A transition a page's table refuses.
pub(crate) type InvalidPageTransition = InvalidTransition<PageState, PageEvent>;

/// What `event` does to a page in `state`: its next state and its effect,
/// or why it is refused.
#[expect(
    clippy::too_many_lines,
    reason = "one table of six states by eighteen events, which ADR 0128 keeps in one function"
)]
pub(crate) fn page(
    state: PageState,
    event: PageEvent,
) -> Result<(PageState, PageEffect), InvalidPageTransition> {
    let refused = |refusal| {
        Err(InvalidTransition {
            state,
            event,
            refusal,
        })
    };
    let late = Ok((state, PageEffect::Nothing));
    match state {
        PageState::Open => match event {
            PageEvent::Gesture | PageEvent::Read => Ok((PageState::Open, PageEffect::Serve)),
            PageEvent::Goto => Ok((PageState::Navigating, PageEffect::Serve)),
            PageEvent::ExternalNav | PageEvent::TargetEvent => {
                Ok((PageState::Open, PageEffect::Nothing))
            }
            PageEvent::Judged => Ok((PageState::Open, PageEffect::Judge)),
            PageEvent::Pause => Ok((PageState::Paused, PageEffect::Pause)),
            PageEvent::Close
            | PageEvent::LeaseEnded
            | PageEvent::MemoryExceeded
            | PageEvent::EventsLost => Ok((PageState::Closing, PageEffect::Close)),
            PageEvent::TargetDestroyed => Ok((PageState::Closed, PageEffect::ClosedInBrowser)),
            PageEvent::RendererCrash => Ok((PageState::Crashed, PageEffect::RendererCrash)),
            PageEvent::BrowserQuit => Ok((PageState::Closed, PageEffect::Quit)),
            PageEvent::BrowserCrash => Ok((PageState::Crashed, PageEffect::BrowserCrash)),
            PageEvent::Committed | PageEvent::NavFailed | PageEvent::PauseEnded => {
                refused(Refusal::Internal)
            }
        },
        PageState::Navigating => match event {
            PageEvent::Gesture | PageEvent::Pause => Ok((PageState::Navigating, PageEffect::Wait)),
            PageEvent::Goto => Ok((PageState::Navigating, PageEffect::Interrupt)),
            PageEvent::Read => Ok((PageState::Navigating, PageEffect::Serve)),
            PageEvent::ExternalNav => Ok((PageState::Open, PageEffect::Interrupt)),
            PageEvent::Committed => Ok((PageState::Open, PageEffect::Commit)),
            PageEvent::NavFailed => Ok((PageState::Open, PageEffect::FailNavigation)),
            PageEvent::TargetEvent => Ok((PageState::Navigating, PageEffect::Nothing)),
            PageEvent::Close
            | PageEvent::LeaseEnded
            | PageEvent::MemoryExceeded
            | PageEvent::EventsLost => Ok((PageState::Closing, PageEffect::Close)),
            PageEvent::TargetDestroyed => Ok((PageState::Closed, PageEffect::ClosedInBrowser)),
            PageEvent::RendererCrash => Ok((PageState::Crashed, PageEffect::RendererCrash)),
            PageEvent::BrowserQuit => Ok((PageState::Closed, PageEffect::Quit)),
            PageEvent::BrowserCrash => Ok((PageState::Crashed, PageEffect::BrowserCrash)),
            PageEvent::Judged | PageEvent::PauseEnded => refused(Refusal::Internal),
        },
        PageState::Paused => match event {
            PageEvent::Gesture | PageEvent::Goto | PageEvent::Pause => {
                Ok((PageState::Paused, PageEffect::Wait))
            }
            PageEvent::Read => Ok((PageState::Paused, PageEffect::Serve)),
            PageEvent::ExternalNav | PageEvent::TargetEvent => {
                Ok((PageState::Paused, PageEffect::Nothing))
            }
            PageEvent::Judged => Ok((PageState::Paused, PageEffect::Defer)),
            PageEvent::PauseEnded => Ok((PageState::Open, PageEffect::EndPause)),
            PageEvent::Close
            | PageEvent::LeaseEnded
            | PageEvent::MemoryExceeded
            | PageEvent::EventsLost => Ok((PageState::Closing, PageEffect::Close)),
            PageEvent::TargetDestroyed => Ok((PageState::Closed, PageEffect::ClosedInBrowser)),
            PageEvent::RendererCrash => Ok((PageState::Crashed, PageEffect::RendererCrash)),
            PageEvent::BrowserQuit => Ok((PageState::Closed, PageEffect::Quit)),
            PageEvent::BrowserCrash => Ok((PageState::Crashed, PageEffect::BrowserCrash)),
            PageEvent::Committed | PageEvent::NavFailed => refused(Refusal::Internal),
        },
        PageState::Crashed => match event {
            PageEvent::Gesture | PageEvent::Read | PageEvent::Goto | PageEvent::Pause => {
                refused(Refusal::Program(PAGE_CRASHED.name))
            }
            PageEvent::Close
            | PageEvent::LeaseEnded
            | PageEvent::MemoryExceeded
            | PageEvent::EventsLost => Ok((PageState::Crashed, PageEffect::Release)),
            PageEvent::ExternalNav
            | PageEvent::Committed
            | PageEvent::NavFailed
            | PageEvent::Judged
            | PageEvent::PauseEnded
            | PageEvent::TargetDestroyed
            | PageEvent::RendererCrash
            | PageEvent::BrowserQuit
            | PageEvent::BrowserCrash
            | PageEvent::TargetEvent => late,
        },
        PageState::Closing => match event {
            PageEvent::Gesture | PageEvent::Read | PageEvent::Goto | PageEvent::Pause => {
                refused(Refusal::Program(CLOSED.name))
            }
            PageEvent::TargetDestroyed => Ok((PageState::Closed, PageEffect::Confirm)),
            PageEvent::BrowserQuit | PageEvent::BrowserCrash => {
                Ok((PageState::Closed, PageEffect::CloseOvertaken))
            }
            PageEvent::ExternalNav
            | PageEvent::Committed
            | PageEvent::NavFailed
            | PageEvent::Judged
            | PageEvent::PauseEnded
            | PageEvent::Close
            | PageEvent::RendererCrash
            | PageEvent::TargetEvent
            | PageEvent::LeaseEnded
            | PageEvent::MemoryExceeded
            | PageEvent::EventsLost => late,
        },
        PageState::Closed => match event {
            PageEvent::Gesture | PageEvent::Read | PageEvent::Goto | PageEvent::Pause => {
                refused(Refusal::Program(CLOSED.name))
            }
            PageEvent::ExternalNav
            | PageEvent::Committed
            | PageEvent::NavFailed
            | PageEvent::Judged
            | PageEvent::PauseEnded
            | PageEvent::Close
            | PageEvent::TargetDestroyed
            | PageEvent::RendererCrash
            | PageEvent::BrowserQuit
            | PageEvent::BrowserCrash
            | PageEvent::TargetEvent
            | PageEvent::LeaseEnded
            | PageEvent::MemoryExceeded
            | PageEvent::EventsLost => late,
        },
    }
}

#[cfg(test)]
mod tests;
