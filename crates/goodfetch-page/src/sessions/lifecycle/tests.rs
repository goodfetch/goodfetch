//! Every pair of a Session's and a page's tables (ADR 0128), each a named
//! case `session::<state>::<event>` or `page::<state>::<event>` whose
//! outcome is copied from spec #594's matrices and catalogues; a Session's
//! cells that its source or owner decides have a case per attribute.

/// Covers: story:5-27, story:5-28
#[test]
fn the_session_and_page_tables_match_their_state_and_event_with_no_wildcard() {
    assert_eq!(
        goodfetch_test_support::refused_matches(include_str!("../lifecycle.rs")),
        Vec::<&str>::new()
    );
}

mod session {
    use goodfetch_engine::transition::Refusal;
    use rstest::rstest;

    use super::super::{
        Attributes, Owner, SessionEffect, SessionEvent, SessionState, Source, Suspended, session,
    };

    type Outcome = Result<(SessionState, SessionEffect), Refusal>;

    const INTERNAL: Outcome = Err(Refusal::Internal);
    const CLOSED: Outcome = Err(Refusal::Program("closed"));
    const PAGE_CRASHED: Outcome = Err(Refusal::Program("page_crashed"));
    const SESSION_RESERVED: Outcome = Err(Refusal::Program("session_reserved"));

    /// A fresh Session a program opened in a browser the engine launched.
    const LAUNCHED: Attributes = Attributes {
        source: Source::Launched,
        owner: Owner::Program,
    };
    /// A Session an Agent opened.
    const AGENTS: Attributes = Attributes {
        source: Source::Launched,
        owner: Owner::Agent,
    };
    /// A Session a person's own Browser host serves.
    const PERSONS_HOST: Attributes = Attributes {
        source: Source::PersonsBrowserHost,
        owner: Owner::Program,
    };
    /// A Session a Browser host an operator runs serves.
    const HOSTED_HOST: Attributes = Attributes {
        source: Source::HostedBrowserHost,
        owner: Owner::Program,
    };
    /// A Session a crawl's Session pool opened.
    const POOL: Attributes = Attributes {
        source: Source::Launched,
        owner: Owner::Pool,
    };
    /// A Session a crawl's Session pool opened on a copied Profile
    /// directory.
    const POOL_PROFILE: Attributes = Attributes {
        source: Source::ProfileDirectory,
        owner: Owner::Pool,
    };
    /// A Session a crawl's Session pool opened on a Borrowed browser.
    const POOL_BORROWED: Attributes = Attributes {
        source: Source::Borrowed,
        owner: Owner::Pool,
    };
    /// A Session a crawl's Session pool opened on a Profile directory used
    /// in place.
    const POOL_IN_PLACE: Attributes = Attributes {
        source: Source::InPlaceProfileDirectory,
        owner: Owner::Pool,
    };

    fn outcome(state: SessionState, event: SessionEvent, attributes: Attributes) -> Outcome {
        session(state, event, attributes).map_err(|invalid| invalid.refusal)
    }

    /// `Opening`'s row of spec #594's Session matrix.
    ///
    /// Covers: story:5-27, story:5-28
    #[rstest]
    #[case::source_ready(SessionEvent::SourceReady, LAUNCHED, Ok((SessionState::Open, SessionEffect::Opened)))]
    #[case::open_failed(SessionEvent::OpenFailed, LAUNCHED, Ok((SessionState::Closed, SessionEffect::FailOpen)))]
    #[case::call(SessionEvent::Call, LAUNCHED, Ok((SessionState::Opening, SessionEffect::Wait)))]
    #[case::lend(SessionEvent::Lend, LAUNCHED, INTERNAL)]
    #[case::attach_ended(SessionEvent::AttachEnded, LAUNCHED, INTERNAL)]
    #[case::close(SessionEvent::Close, LAUNCHED, Ok((SessionState::Closing, SessionEffect::AbandonOpen)))]
    #[case::children_closed(SessionEvent::ChildrenClosed, LAUNCHED, INTERNAL)]
    #[case::browser_quit(SessionEvent::BrowserQuit, LAUNCHED, Ok((SessionState::Closed, SessionEffect::FailOpen)))]
    #[case::browser_crash(SessionEvent::BrowserCrash, LAUNCHED, Ok((SessionState::Closed, SessionEffect::FailOpen)))]
    #[case::host_offline(SessionEvent::HostOffline, PERSONS_HOST, Ok((SessionState::Offline(Suspended::Opening), SessionEffect::Suspend)))]
    #[case::host_offline_on_a_hosted_browser_host(SessionEvent::HostOffline, HOSTED_HOST, Ok((SessionState::Closed, SessionEffect::FailOpen)))]
    #[case::host_offline_on_a_launched_browser(SessionEvent::HostOffline, LAUNCHED, INTERNAL)]
    #[case::host_back(SessionEvent::HostBack { browser_quit: false }, LAUNCHED, Ok((SessionState::Opening, SessionEffect::Nothing)))]
    #[case::retire(SessionEvent::Retire, LAUNCHED, INTERNAL)]
    fn opening(
        #[case] event: SessionEvent,
        #[case] attributes: Attributes,
        #[case] expected: Outcome,
    ) {
        assert_eq!(outcome(SessionState::Opening, event, attributes), expected);
    }

    /// `Open`'s row of spec #594's Session matrix.
    ///
    /// Covers: story:5-27, story:5-28
    #[rstest]
    #[case::source_ready(SessionEvent::SourceReady, LAUNCHED, INTERNAL)]
    #[case::open_failed(SessionEvent::OpenFailed, LAUNCHED, INTERNAL)]
    #[case::call(SessionEvent::Call, LAUNCHED, Ok((SessionState::Open, SessionEffect::Serve)))]
    #[case::lend(SessionEvent::Lend, LAUNCHED, Ok((SessionState::Lent, SessionEffect::Lend)))]
    #[case::attach_ended(SessionEvent::AttachEnded, LAUNCHED, INTERNAL)]
    #[case::close(SessionEvent::Close, LAUNCHED, Ok((SessionState::Closing, SessionEffect::CloseChildren)))]
    #[case::children_closed(SessionEvent::ChildrenClosed, LAUNCHED, INTERNAL)]
    #[case::browser_quit(SessionEvent::BrowserQuit, LAUNCHED, Ok((SessionState::Closed, SessionEffect::Quit)))]
    #[case::browser_crash(SessionEvent::BrowserCrash, LAUNCHED, Ok((SessionState::Crashed, SessionEffect::Crash)))]
    #[case::host_offline(SessionEvent::HostOffline, PERSONS_HOST, Ok((SessionState::Offline(Suspended::Open), SessionEffect::Suspend)))]
    #[case::host_offline_on_a_hosted_browser_host(SessionEvent::HostOffline, HOSTED_HOST, Ok((SessionState::Open, SessionEffect::Unreachable)))]
    #[case::host_offline_on_a_launched_browser(SessionEvent::HostOffline, LAUNCHED, INTERNAL)]
    #[case::host_back(SessionEvent::HostBack { browser_quit: false }, LAUNCHED, Ok((SessionState::Open, SessionEffect::Nothing)))]
    #[case::retire(SessionEvent::Retire, POOL, Ok((SessionState::Closing, SessionEffect::CloseChildren)))]
    #[case::retire_of_a_programs_session(SessionEvent::Retire, LAUNCHED, INTERNAL)]
    #[case::retire_of_an_agents_session(SessionEvent::Retire, AGENTS, INTERNAL)]
    #[case::retire_of_a_pools_copied_profile_directory(SessionEvent::Retire, POOL_PROFILE, Ok((SessionState::Closing, SessionEffect::CloseChildren)))]
    #[case::retire_of_a_pools_borrowed_browser(SessionEvent::Retire, POOL_BORROWED, INTERNAL)]
    #[case::retire_of_a_pools_profile_directory_in_place(
        SessionEvent::Retire,
        POOL_IN_PLACE,
        INTERNAL
    )]
    fn open(
        #[case] event: SessionEvent,
        #[case] attributes: Attributes,
        #[case] expected: Outcome,
    ) {
        assert_eq!(outcome(SessionState::Open, event, attributes), expected);
    }

    /// `Lent`'s row of spec #594's Session matrix.
    ///
    /// Covers: story:5-27, story:5-28
    #[rstest]
    #[case::source_ready(SessionEvent::SourceReady, LAUNCHED, INTERNAL)]
    #[case::open_failed(SessionEvent::OpenFailed, LAUNCHED, INTERNAL)]
    #[case::call(SessionEvent::Call, LAUNCHED, Ok((SessionState::Lent, SessionEffect::Serve)))]
    #[case::lend(SessionEvent::Lend, LAUNCHED, SESSION_RESERVED)]
    #[case::attach_ended(SessionEvent::AttachEnded, LAUNCHED, Ok((SessionState::Open, SessionEffect::EndAttach)))]
    #[case::close(SessionEvent::Close, LAUNCHED, Ok((SessionState::Closing, SessionEffect::EndAttachAndClose)))]
    #[case::children_closed(SessionEvent::ChildrenClosed, LAUNCHED, INTERNAL)]
    #[case::browser_quit(SessionEvent::BrowserQuit, LAUNCHED, Ok((SessionState::Closed, SessionEffect::Quit)))]
    #[case::browser_crash(SessionEvent::BrowserCrash, LAUNCHED, Ok((SessionState::Crashed, SessionEffect::Crash)))]
    #[case::host_offline(SessionEvent::HostOffline, PERSONS_HOST, Ok((SessionState::Offline(Suspended::Open), SessionEffect::EndAttachAndSuspend)))]
    #[case::host_offline_on_a_hosted_browser_host(SessionEvent::HostOffline, HOSTED_HOST, Ok((SessionState::Lent, SessionEffect::Unreachable)))]
    #[case::host_offline_on_a_launched_browser(SessionEvent::HostOffline, LAUNCHED, INTERNAL)]
    #[case::host_back(SessionEvent::HostBack { browser_quit: false }, LAUNCHED, Ok((SessionState::Lent, SessionEffect::Nothing)))]
    #[case::retire(SessionEvent::Retire, LAUNCHED, INTERNAL)]
    fn lent(
        #[case] event: SessionEvent,
        #[case] attributes: Attributes,
        #[case] expected: Outcome,
    ) {
        assert_eq!(outcome(SessionState::Lent, event, attributes), expected);
    }

    /// `Offline`'s row of spec #594's Session matrix.
    ///
    /// Covers: story:5-27, story:5-28
    #[rstest]
    #[case::source_ready(SessionEvent::SourceReady, LAUNCHED, Ok((SessionState::Offline(Suspended::Open), SessionEffect::Nothing)))]
    #[case::open_failed(SessionEvent::OpenFailed, LAUNCHED, Ok((SessionState::Offline(Suspended::Open), SessionEffect::Nothing)))]
    #[case::call(SessionEvent::Call, PERSONS_HOST, Ok((SessionState::Offline(Suspended::Open), SessionEffect::Wait)))]
    #[case::lend(SessionEvent::Lend, PERSONS_HOST, Ok((SessionState::Offline(Suspended::Open), SessionEffect::Wait)))]
    #[case::attach_ended(SessionEvent::AttachEnded, LAUNCHED, INTERNAL)]
    #[case::close(SessionEvent::Close, PERSONS_HOST, Ok((SessionState::Offline(Suspended::Open), SessionEffect::Wait)))]
    #[case::children_closed(SessionEvent::ChildrenClosed, LAUNCHED, INTERNAL)]
    #[case::browser_quit(SessionEvent::BrowserQuit, LAUNCHED, INTERNAL)]
    #[case::browser_crash(SessionEvent::BrowserCrash, LAUNCHED, INTERNAL)]
    #[case::host_offline(SessionEvent::HostOffline, LAUNCHED, INTERNAL)]
    #[case::host_back(SessionEvent::HostBack { browser_quit: false }, PERSONS_HOST, Ok((SessionState::Open, SessionEffect::Resume)))]
    #[case::retire(SessionEvent::Retire, LAUNCHED, INTERNAL)]
    #[case::host_back_after_its_browser_quit(SessionEvent::HostBack { browser_quit: true }, PERSONS_HOST, Ok((SessionState::Closed, SessionEffect::Quit)))]
    fn offline(
        #[case] event: SessionEvent,
        #[case] attributes: Attributes,
        #[case] expected: Outcome,
    ) {
        assert_eq!(
            outcome(SessionState::Offline(Suspended::Open), event, attributes),
            expected
        );
    }

    /// An `Offline` Session whose link was lost while it was `Opening`
    /// opens again on the returned link (S18, S25).
    ///
    /// Covers: story:5-27, story:5-28
    #[rstest]
    #[case::host_back(SessionEvent::HostBack { browser_quit: false }, Ok((SessionState::Opening, SessionEffect::Reopen)))]
    #[case::host_back_after_its_browser_quit(SessionEvent::HostBack { browser_quit: true }, Ok((SessionState::Closed, SessionEffect::Quit)))]
    fn offline_while_opening(#[case] event: SessionEvent, #[case] expected: Outcome) {
        assert_eq!(
            outcome(
                SessionState::Offline(Suspended::Opening),
                event,
                PERSONS_HOST
            ),
            expected
        );
    }

    /// `Closing`'s row of spec #594's Session matrix.
    ///
    /// Covers: story:5-27, story:5-28
    #[rstest]
    #[case::source_ready(SessionEvent::SourceReady, LAUNCHED, Ok((SessionState::Closing, SessionEffect::Nothing)))]
    #[case::open_failed(SessionEvent::OpenFailed, LAUNCHED, Ok((SessionState::Closing, SessionEffect::Nothing)))]
    #[case::call(SessionEvent::Call, LAUNCHED, CLOSED)]
    #[case::lend(SessionEvent::Lend, LAUNCHED, CLOSED)]
    #[case::attach_ended(SessionEvent::AttachEnded, LAUNCHED, Ok((SessionState::Closing, SessionEffect::Nothing)))]
    #[case::close(SessionEvent::Close, LAUNCHED, Ok((SessionState::Closing, SessionEffect::Nothing)))]
    #[case::children_closed(SessionEvent::ChildrenClosed, LAUNCHED, Ok((SessionState::Closed, SessionEffect::Release)))]
    #[case::browser_quit(SessionEvent::BrowserQuit, LAUNCHED, Ok((SessionState::Closing, SessionEffect::Nothing)))]
    #[case::browser_crash(SessionEvent::BrowserCrash, LAUNCHED, Ok((SessionState::Closing, SessionEffect::Nothing)))]
    #[case::host_offline(SessionEvent::HostOffline, LAUNCHED, Ok((SessionState::Closing, SessionEffect::Nothing)))]
    #[case::host_back(SessionEvent::HostBack { browser_quit: false }, LAUNCHED, Ok((SessionState::Closing, SessionEffect::Nothing)))]
    #[case::retire(SessionEvent::Retire, LAUNCHED, Ok((SessionState::Closing, SessionEffect::Nothing)))]
    fn closing(
        #[case] event: SessionEvent,
        #[case] attributes: Attributes,
        #[case] expected: Outcome,
    ) {
        assert_eq!(outcome(SessionState::Closing, event, attributes), expected);
    }

    /// `Closed`'s row of spec #594's Session matrix.
    ///
    /// Covers: story:5-27, story:5-28
    #[rstest]
    #[case::source_ready(SessionEvent::SourceReady, LAUNCHED, Ok((SessionState::Closed, SessionEffect::Nothing)))]
    #[case::open_failed(SessionEvent::OpenFailed, LAUNCHED, Ok((SessionState::Closed, SessionEffect::Nothing)))]
    #[case::call(SessionEvent::Call, LAUNCHED, CLOSED)]
    #[case::lend(SessionEvent::Lend, LAUNCHED, CLOSED)]
    #[case::attach_ended(SessionEvent::AttachEnded, LAUNCHED, Ok((SessionState::Closed, SessionEffect::Nothing)))]
    #[case::close(SessionEvent::Close, LAUNCHED, Ok((SessionState::Closed, SessionEffect::Nothing)))]
    #[case::children_closed(SessionEvent::ChildrenClosed, LAUNCHED, INTERNAL)]
    #[case::browser_quit(SessionEvent::BrowserQuit, LAUNCHED, Ok((SessionState::Closed, SessionEffect::Nothing)))]
    #[case::browser_crash(SessionEvent::BrowserCrash, LAUNCHED, Ok((SessionState::Closed, SessionEffect::Nothing)))]
    #[case::host_offline(SessionEvent::HostOffline, LAUNCHED, Ok((SessionState::Closed, SessionEffect::Nothing)))]
    #[case::host_back(SessionEvent::HostBack { browser_quit: false }, LAUNCHED, Ok((SessionState::Closed, SessionEffect::Nothing)))]
    #[case::retire(SessionEvent::Retire, LAUNCHED, Ok((SessionState::Closed, SessionEffect::Nothing)))]
    fn closed(
        #[case] event: SessionEvent,
        #[case] attributes: Attributes,
        #[case] expected: Outcome,
    ) {
        assert_eq!(outcome(SessionState::Closed, event, attributes), expected);
    }

    /// `Crashed`'s row of spec #594's Session matrix.
    ///
    /// Covers: story:5-27, story:5-28
    #[rstest]
    #[case::source_ready(SessionEvent::SourceReady, LAUNCHED, INTERNAL)]
    #[case::open_failed(SessionEvent::OpenFailed, LAUNCHED, INTERNAL)]
    #[case::call(SessionEvent::Call, LAUNCHED, PAGE_CRASHED)]
    #[case::lend(SessionEvent::Lend, LAUNCHED, PAGE_CRASHED)]
    #[case::attach_ended(SessionEvent::AttachEnded, LAUNCHED, Ok((SessionState::Crashed, SessionEffect::Nothing)))]
    #[case::close(SessionEvent::Close, LAUNCHED, Ok((SessionState::Crashed, SessionEffect::Release)))]
    #[case::children_closed(SessionEvent::ChildrenClosed, LAUNCHED, INTERNAL)]
    #[case::browser_quit(SessionEvent::BrowserQuit, LAUNCHED, Ok((SessionState::Crashed, SessionEffect::Nothing)))]
    #[case::browser_crash(SessionEvent::BrowserCrash, LAUNCHED, Ok((SessionState::Crashed, SessionEffect::Nothing)))]
    #[case::host_offline(SessionEvent::HostOffline, LAUNCHED, Ok((SessionState::Crashed, SessionEffect::Nothing)))]
    #[case::host_back(SessionEvent::HostBack { browser_quit: false }, LAUNCHED, Ok((SessionState::Crashed, SessionEffect::Nothing)))]
    #[case::retire(SessionEvent::Retire, LAUNCHED, Ok((SessionState::Crashed, SessionEffect::Nothing)))]
    fn crashed(
        #[case] event: SessionEvent,
        #[case] attributes: Attributes,
        #[case] expected: Outcome,
    ) {
        assert_eq!(outcome(SessionState::Crashed, event, attributes), expected);
    }
}

mod page {
    use goodfetch_engine::transition::Refusal;
    use rstest::rstest;

    use super::super::{PageEffect, PageEvent, PageState, page};

    type Outcome = Result<(PageState, PageEffect), Refusal>;

    const INTERNAL: Outcome = Err(Refusal::Internal);
    const CLOSED: Outcome = Err(Refusal::Program("closed"));
    const PAGE_CRASHED: Outcome = Err(Refusal::Program("page_crashed"));

    fn outcome(state: PageState, event: PageEvent) -> Outcome {
        page(state, event).map_err(|invalid| invalid.refusal)
    }

    /// `Open`'s row of spec #594's page matrix.
    ///
    /// Covers: story:5-27, story:5-28
    #[rstest]
    #[case::gesture(PageEvent::Gesture, Ok((PageState::Open, PageEffect::Serve)))]
    #[case::read(PageEvent::Read, Ok((PageState::Open, PageEffect::Serve)))]
    #[case::goto(PageEvent::Goto, Ok((PageState::Navigating, PageEffect::Serve)))]
    #[case::external_nav(PageEvent::ExternalNav, Ok((PageState::Open, PageEffect::Nothing)))]
    #[case::committed(PageEvent::Committed, INTERNAL)]
    #[case::nav_failed(PageEvent::NavFailed, INTERNAL)]
    #[case::judged(PageEvent::Judged, Ok((PageState::Open, PageEffect::Judge)))]
    #[case::pause(PageEvent::Pause, Ok((PageState::Paused, PageEffect::Pause)))]
    #[case::pause_ended(PageEvent::PauseEnded, INTERNAL)]
    #[case::close(PageEvent::Close, Ok((PageState::Closing, PageEffect::Close)))]
    #[case::target_destroyed(PageEvent::TargetDestroyed, Ok((PageState::Closed, PageEffect::ClosedInBrowser)))]
    #[case::renderer_crash(PageEvent::RendererCrash, Ok((PageState::Crashed, PageEffect::RendererCrash)))]
    #[case::browser_quit(PageEvent::BrowserQuit, Ok((PageState::Closed, PageEffect::Quit)))]
    #[case::browser_crash(PageEvent::BrowserCrash, Ok((PageState::Crashed, PageEffect::BrowserCrash)))]
    #[case::target_event(PageEvent::TargetEvent, Ok((PageState::Open, PageEffect::Nothing)))]
    #[case::lease_ended(PageEvent::LeaseEnded, Ok((PageState::Closing, PageEffect::Close)))]
    #[case::memory_exceeded(PageEvent::MemoryExceeded, Ok((PageState::Closing, PageEffect::Close)))]
    fn open(#[case] event: PageEvent, #[case] expected: Outcome) {
        assert_eq!(outcome(PageState::Open, event), expected);
    }

    /// `Navigating`'s row of spec #594's page matrix.
    ///
    /// Covers: story:5-27, story:5-28
    #[rstest]
    #[case::gesture(PageEvent::Gesture, Ok((PageState::Navigating, PageEffect::Wait)))]
    #[case::read(PageEvent::Read, Ok((PageState::Navigating, PageEffect::Serve)))]
    #[case::goto(PageEvent::Goto, Ok((PageState::Navigating, PageEffect::Interrupt)))]
    #[case::external_nav(PageEvent::ExternalNav, Ok((PageState::Open, PageEffect::Interrupt)))]
    #[case::committed(PageEvent::Committed, Ok((PageState::Open, PageEffect::Commit)))]
    #[case::nav_failed(PageEvent::NavFailed, Ok((PageState::Open, PageEffect::FailNavigation)))]
    #[case::judged(PageEvent::Judged, INTERNAL)]
    #[case::pause(PageEvent::Pause, Ok((PageState::Navigating, PageEffect::Wait)))]
    #[case::pause_ended(PageEvent::PauseEnded, INTERNAL)]
    #[case::close(PageEvent::Close, Ok((PageState::Closing, PageEffect::Close)))]
    #[case::target_destroyed(PageEvent::TargetDestroyed, Ok((PageState::Closed, PageEffect::ClosedInBrowser)))]
    #[case::renderer_crash(PageEvent::RendererCrash, Ok((PageState::Crashed, PageEffect::RendererCrash)))]
    #[case::browser_quit(PageEvent::BrowserQuit, Ok((PageState::Closed, PageEffect::Quit)))]
    #[case::browser_crash(PageEvent::BrowserCrash, Ok((PageState::Crashed, PageEffect::BrowserCrash)))]
    #[case::target_event(PageEvent::TargetEvent, Ok((PageState::Navigating, PageEffect::Nothing)))]
    #[case::lease_ended(PageEvent::LeaseEnded, Ok((PageState::Closing, PageEffect::Close)))]
    #[case::memory_exceeded(PageEvent::MemoryExceeded, Ok((PageState::Closing, PageEffect::Close)))]
    fn navigating(#[case] event: PageEvent, #[case] expected: Outcome) {
        assert_eq!(outcome(PageState::Navigating, event), expected);
    }

    /// `Paused`'s row of spec #594's page matrix.
    ///
    /// Covers: story:5-27, story:5-28
    #[rstest]
    #[case::gesture(PageEvent::Gesture, Ok((PageState::Paused, PageEffect::Wait)))]
    #[case::read(PageEvent::Read, Ok((PageState::Paused, PageEffect::Serve)))]
    #[case::goto(PageEvent::Goto, Ok((PageState::Paused, PageEffect::Wait)))]
    #[case::external_nav(PageEvent::ExternalNav, Ok((PageState::Paused, PageEffect::Nothing)))]
    #[case::committed(PageEvent::Committed, INTERNAL)]
    #[case::nav_failed(PageEvent::NavFailed, INTERNAL)]
    #[case::judged(PageEvent::Judged, Ok((PageState::Paused, PageEffect::Defer)))]
    #[case::pause(PageEvent::Pause, Ok((PageState::Paused, PageEffect::Wait)))]
    #[case::pause_ended(PageEvent::PauseEnded, Ok((PageState::Open, PageEffect::EndPause)))]
    #[case::close(PageEvent::Close, Ok((PageState::Closing, PageEffect::Close)))]
    #[case::target_destroyed(PageEvent::TargetDestroyed, Ok((PageState::Closed, PageEffect::ClosedInBrowser)))]
    #[case::renderer_crash(PageEvent::RendererCrash, Ok((PageState::Crashed, PageEffect::RendererCrash)))]
    #[case::browser_quit(PageEvent::BrowserQuit, Ok((PageState::Closed, PageEffect::Quit)))]
    #[case::browser_crash(PageEvent::BrowserCrash, Ok((PageState::Crashed, PageEffect::BrowserCrash)))]
    #[case::target_event(PageEvent::TargetEvent, Ok((PageState::Paused, PageEffect::Nothing)))]
    #[case::lease_ended(PageEvent::LeaseEnded, Ok((PageState::Closing, PageEffect::Close)))]
    #[case::memory_exceeded(PageEvent::MemoryExceeded, Ok((PageState::Closing, PageEffect::Close)))]
    fn paused(#[case] event: PageEvent, #[case] expected: Outcome) {
        assert_eq!(outcome(PageState::Paused, event), expected);
    }

    /// `Crashed`'s row of spec #594's page matrix.
    ///
    /// Covers: story:5-27, story:5-28
    #[rstest]
    #[case::gesture(PageEvent::Gesture, PAGE_CRASHED)]
    #[case::read(PageEvent::Read, PAGE_CRASHED)]
    #[case::goto(PageEvent::Goto, PAGE_CRASHED)]
    #[case::external_nav(PageEvent::ExternalNav, Ok((PageState::Crashed, PageEffect::Nothing)))]
    #[case::committed(PageEvent::Committed, Ok((PageState::Crashed, PageEffect::Nothing)))]
    #[case::nav_failed(PageEvent::NavFailed, Ok((PageState::Crashed, PageEffect::Nothing)))]
    #[case::judged(PageEvent::Judged, Ok((PageState::Crashed, PageEffect::Nothing)))]
    #[case::pause(PageEvent::Pause, PAGE_CRASHED)]
    #[case::pause_ended(PageEvent::PauseEnded, Ok((PageState::Crashed, PageEffect::Nothing)))]
    #[case::close(PageEvent::Close, Ok((PageState::Crashed, PageEffect::Release)))]
    #[case::target_destroyed(PageEvent::TargetDestroyed, Ok((PageState::Crashed, PageEffect::Nothing)))]
    #[case::renderer_crash(PageEvent::RendererCrash, Ok((PageState::Crashed, PageEffect::Nothing)))]
    #[case::browser_quit(PageEvent::BrowserQuit, Ok((PageState::Crashed, PageEffect::Nothing)))]
    #[case::browser_crash(PageEvent::BrowserCrash, Ok((PageState::Crashed, PageEffect::Nothing)))]
    #[case::target_event(PageEvent::TargetEvent, Ok((PageState::Crashed, PageEffect::Nothing)))]
    #[case::lease_ended(PageEvent::LeaseEnded, Ok((PageState::Crashed, PageEffect::Release)))]
    #[case::memory_exceeded(PageEvent::MemoryExceeded, Ok((PageState::Crashed, PageEffect::Release)))]
    fn crashed(#[case] event: PageEvent, #[case] expected: Outcome) {
        assert_eq!(outcome(PageState::Crashed, event), expected);
    }

    /// `Closing`'s row of spec #594's page matrix.
    ///
    /// Covers: story:5-27, story:5-28
    #[rstest]
    #[case::gesture(PageEvent::Gesture, CLOSED)]
    #[case::read(PageEvent::Read, CLOSED)]
    #[case::goto(PageEvent::Goto, CLOSED)]
    #[case::external_nav(PageEvent::ExternalNav, Ok((PageState::Closing, PageEffect::Nothing)))]
    #[case::committed(PageEvent::Committed, Ok((PageState::Closing, PageEffect::Nothing)))]
    #[case::nav_failed(PageEvent::NavFailed, Ok((PageState::Closing, PageEffect::Nothing)))]
    #[case::judged(PageEvent::Judged, Ok((PageState::Closing, PageEffect::Nothing)))]
    #[case::pause(PageEvent::Pause, CLOSED)]
    #[case::pause_ended(PageEvent::PauseEnded, Ok((PageState::Closing, PageEffect::Nothing)))]
    #[case::close(PageEvent::Close, Ok((PageState::Closing, PageEffect::Nothing)))]
    #[case::target_destroyed(PageEvent::TargetDestroyed, Ok((PageState::Closed, PageEffect::Confirm)))]
    #[case::renderer_crash(PageEvent::RendererCrash, Ok((PageState::Closing, PageEffect::Nothing)))]
    #[case::browser_quit(PageEvent::BrowserQuit, Ok((PageState::Closed, PageEffect::CloseOvertaken)))]
    #[case::browser_crash(PageEvent::BrowserCrash, Ok((PageState::Closed, PageEffect::CloseOvertaken)))]
    #[case::target_event(PageEvent::TargetEvent, Ok((PageState::Closing, PageEffect::Nothing)))]
    #[case::lease_ended(PageEvent::LeaseEnded, Ok((PageState::Closing, PageEffect::Nothing)))]
    #[case::memory_exceeded(PageEvent::MemoryExceeded, Ok((PageState::Closing, PageEffect::Nothing)))]
    fn closing(#[case] event: PageEvent, #[case] expected: Outcome) {
        assert_eq!(outcome(PageState::Closing, event), expected);
    }

    /// `Closed`'s row of spec #594's page matrix.
    ///
    /// Covers: story:5-27, story:5-28
    #[rstest]
    #[case::gesture(PageEvent::Gesture, CLOSED)]
    #[case::read(PageEvent::Read, CLOSED)]
    #[case::goto(PageEvent::Goto, CLOSED)]
    #[case::external_nav(PageEvent::ExternalNav, Ok((PageState::Closed, PageEffect::Nothing)))]
    #[case::committed(PageEvent::Committed, Ok((PageState::Closed, PageEffect::Nothing)))]
    #[case::nav_failed(PageEvent::NavFailed, Ok((PageState::Closed, PageEffect::Nothing)))]
    #[case::judged(PageEvent::Judged, Ok((PageState::Closed, PageEffect::Nothing)))]
    #[case::pause(PageEvent::Pause, CLOSED)]
    #[case::pause_ended(PageEvent::PauseEnded, Ok((PageState::Closed, PageEffect::Nothing)))]
    #[case::close(PageEvent::Close, Ok((PageState::Closed, PageEffect::Nothing)))]
    #[case::target_destroyed(PageEvent::TargetDestroyed, Ok((PageState::Closed, PageEffect::Nothing)))]
    #[case::renderer_crash(PageEvent::RendererCrash, Ok((PageState::Closed, PageEffect::Nothing)))]
    #[case::browser_quit(PageEvent::BrowserQuit, Ok((PageState::Closed, PageEffect::Nothing)))]
    #[case::browser_crash(PageEvent::BrowserCrash, Ok((PageState::Closed, PageEffect::Nothing)))]
    #[case::target_event(PageEvent::TargetEvent, Ok((PageState::Closed, PageEffect::Nothing)))]
    #[case::lease_ended(PageEvent::LeaseEnded, Ok((PageState::Closed, PageEffect::Nothing)))]
    #[case::memory_exceeded(PageEvent::MemoryExceeded, Ok((PageState::Closed, PageEffect::Nothing)))]
    fn closed(#[case] event: PageEvent, #[case] expected: Outcome) {
        assert_eq!(outcome(PageState::Closed, event), expected);
    }
}

/// A local address is held only by a browser the engine launches on this
/// machine (ADR 0044): a Borrowed browser, a third-party CDP browser among
/// them, and either kind of Browser host refuse it.
///
/// Covers: story:4-48
#[rstest::rstest]
#[case::launched(Kind::Launched, true)]
#[case::profile_directory(Kind::ProfileDirectory, true)]
#[case::in_place_profile_directory(Kind::InPlaceProfileDirectory, true)]
#[case::borrowed(Kind::Borrowed, false)]
#[case::persons_browser_host(Kind::PersonsBrowserHost, false)]
#[case::hosted_browser_host(Kind::HostedBrowserHost, false)]
fn only_a_browser_launched_here_holds_a_local_address(#[case] kind: Kind, #[case] holds: bool) {
    assert_eq!(kind.launched_here(), holds);
}

use super::Source as Kind;
