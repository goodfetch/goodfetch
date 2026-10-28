//! Live hiding (ADR 0047; spec #577): the elements the cosmetic rules of a
//! Session's Filter lists match are hidden on its live pages, on Chromium
//! through the page's hiding sheets ([`Sheets`]), one per document, into
//! which the Consent prompt runner writes its own rules too.
//! Each frame of a page gets its rules in every new document: when the
//! frame commits a document, and again at its `DOMContentLoaded` and its
//! `load`, the guard's isolated world reads the document's URL and the
//! classes and IDs its elements hold, and the sheet is written with the
//! rules they key (`goodfetch_clean_up_rules::hiding`). When the guard
//! finds an action's target not visible because a rule hid it, that rule
//! is lifted for the whole Session, its pages' sheets are written again
//! without it, and Provenance records it; this is never Drift (ADR 0009).
//! In each new document the rules wait until the Consent prompt runner's
//! first detection pass and wall test there ended, so no cosmetic rule
//! hides a consent wall before it is judged (ADR 0048), or at most
//! [`RUNNER_WAIT`] for a document the runner never settles in.
//!
//! `live_view.hiding` chooses: `auto`, the default, and `always` hide (on
//! Firefox and `WebKit`, where any hiding is visible, `auto` will hide only
//! on demand, A05-45); `off` enables neither the `DOM` nor the `CSS`
//! domain, so their cost goes too. Provenance records the choice for each
//! page.

mod documents;
mod sheets;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use goodfetch_cdp::generated::{css, dom, page};
use goodfetch_cdp::{Failure, Subscription};
use goodfetch_clean_up_rules::cosmetic::{CssRule, PageRules};
use goodfetch_clean_up_rules::hiding;
use goodfetch_engine::CallError;
use goodfetch_engine::runs::Run;
use goodfetch_protocol::page::{PageId, SessionId};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::driver::{PageDriver, Turn, Unanswered};
use crate::interception::{Cosmetic, PageCleanUp};
use crate::trace;

pub(crate) use self::documents::Part;
pub(crate) use self::sheets::Sheets;

/// The read of a frame's URL, classes and IDs.
const COLLECT: &str = include_str!("hiding/collect.js");

/// The read of which rules hide a target.
const HIDDEN_BY: &str = include_str!("hiding/hidden_by.js");

/// The longest live hiding waits in a new document for the Consent prompt
/// runner's first pass to end: past autoconsent's own 3 s fallback to the
/// heuristics and their wall test.
const RUNNER_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// What `live_view.hiding` chooses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Auto,
    Off,
    Always,
}

impl Mode {
    /// The mode `name` names; `auto` for any other, as the setting's
    /// validation lets no other through.
    fn of(name: &str) -> Self {
        match name {
            "off" => Self::Off,
            "always" => Self::Always,
            _ => Self::Auto,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Off => "off",
            Self::Always => "always",
        }
    }
}

/// The engine's live hiding: each Session's lifted rules and its pages.
/// Clones share it.
#[derive(Clone, Default)]
pub(crate) struct Hiding {
    sessions: Arc<Mutex<BTreeMap<SessionId, Arc<SessionHiding>>>>,
}

/// One Session's side: the rules the guard lifted for it, and its pages
/// under live hiding.
#[derive(Default)]
struct SessionHiding {
    lifted: Mutex<BTreeSet<String>>,
    pages: Mutex<BTreeMap<PageId, Weak<PageHiding>>>,
}

/// One page under live hiding.
pub(crate) struct PageHiding {
    page_id: PageId,
    session_id: SessionId,
    driver: PageDriver,
    run: Run,
    cosmetic: Cosmetic,
    sheets: Arc<Sheets>,
    session: Arc<SessionHiding>,
    /// The rules each frame's sheet holds now, with the order of the
    /// refresh that wrote them.
    frames: Mutex<BTreeMap<page::FrameId, (u64, Vec<CssRule>)>>,
    /// The order of the next refresh of a frame's sheet.
    refreshes: AtomicU64,
}

/// What [`COLLECT`] answers.
#[derive(Deserialize)]
struct Collected {
    url: String,
    classes: Vec<String>,
    ids: Vec<String>,
}

impl Hiding {
    /// Puts the page `page_id` of the Session `session_id`, which `driver`
    /// drives for `run`, under live hiding as `clean_up` chose, writing into
    /// the page's `sheets` and recording the choice in Provenance; none
    /// under `off` or with no Filter list on.
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses the `DOM` or
    /// the `CSS` domain.
    pub(crate) async fn start(
        &self,
        page_id: &PageId,
        session_id: &SessionId,
        driver: &PageDriver,
        run: &Run,
        clean_up: &PageCleanUp,
        sheets: &Arc<Sheets>,
    ) -> Result<Option<Arc<PageHiding>>, CallError> {
        let mode = Mode::of(&clean_up.live_hiding);
        let cosmetic = clean_up.cosmetic.clone().filter(|_| mode != Mode::Off);
        trace::provenance(
            run,
            &json!({
                "action": "live_hiding",
                "page_id": page_id,
                "session_id": session_id,
                "mode": mode.name(),
                "applied": cosmetic.is_some(),
            }),
        )
        .await;
        let Some(cosmetic) = cosmetic else {
            return Ok(None);
        };
        let session = Arc::clone(
            self.sessions
                .lock()
                .await
                .entry(session_id.clone())
                .or_default(),
        );
        let page = Arc::new(PageHiding {
            page_id: page_id.clone(),
            session_id: session_id.clone(),
            driver: driver.clone(),
            run: run.clone(),
            cosmetic,
            sheets: Arc::clone(sheets),
            session: Arc::clone(&session),
            frames: Mutex::new(BTreeMap::new()),
            refreshes: AtomicU64::new(0),
        });
        session
            .pages
            .lock()
            .await
            .insert(page_id.clone(), Arc::downgrade(&page));
        let events = driver.events();
        driver
            .opening_command(&dom::EnableParams::default())
            .await?;
        driver.opening_command(&css::EnableParams {}).await?;
        sheets.styled(driver.session());
        run.spawn(follow(events, Arc::clone(&page)));
        Ok(Some(page))
    }

    /// Takes the Session `session_id` out of live hiding, as it closes or
    /// its Run stops.
    pub(crate) async fn close(&self, session_id: &SessionId) {
        self.sessions.lock().await.remove(session_id);
    }
}

/// Reads the page's events until its target goes, writing a frame's sheet
/// again, as a task of its own, each time the frame commits a document or
/// reaches its `DOMContentLoaded` or its `load`.
async fn follow(mut events: Subscription, page: Arc<PageHiding>) {
    while let Some(event) = events.next().await {
        let frame = if let Some(navigated) = event.read::<page::FrameNavigatedEvent>() {
            page.sheets.navigated(
                page.driver.session(),
                &navigated.frame.id,
                &navigated.frame.loader_id,
            );
            let waited = Arc::clone(&page);
            let (frame, loader) = (
                navigated.frame.id.clone(),
                navigated.frame.loader_id.clone(),
            );
            page.run.spawn(async move {
                // The Run's end does not wait out the runner's pass.
                let clock = waited.run.clock();
                tokio::select! {
                    () = waited.run.cancellation().cancelled() => return,
                    () = clock.sleep(RUNNER_WAIT) => {}
                }
                let released = waited
                    .sheets
                    .release(waited.driver.session(), &frame, Some(&loader))
                    .await;
                if let Err(failure) = released {
                    tracing::debug!(%failure, "a document's live hiding did not start: its document went");
                }
            });
            navigated.frame.id
        } else if let Some(reached) = event.read::<page::LifecycleEventEvent>()
            && matches!(reached.name.as_str(), "DOMContentLoaded" | "load")
        {
            reached.frame_id
        } else {
            continue;
        };
        let page = Arc::clone(&page);
        let run = page.run.clone();
        run.spawn(async move { page.refresh(&frame).await });
    }
}

impl PageHiding {
    /// Writes the sheet of `frame`'s document as the document stands, but
    /// for the rules lifted for the Session. Refreshes of one frame may run
    /// at once: the sheets send the last one begun, and the frame keeps its
    /// rules, never an earlier one's.
    async fn refresh(&self, frame: &page::FrameId) {
        let order = self.refreshes.fetch_add(1, Ordering::AcqRel);
        let lifted = self.session.lifted.lock().await.clone();
        let written = self.write(frame, &lifted, order).await;
        let mut frames = self.frames.lock().await;
        if frames.get(frame).is_some_and(|(newest, _)| *newest > order) {
            return;
        }
        match written {
            Ok(Some(rules)) => {
                frames.insert(frame.clone(), (order, rules));
            }
            Ok(None) => {
                frames.remove(frame);
            }
            Err(failure) => {
                tracing::debug!(%failure, "a frame's live-hiding sheet was not written: its document went");
                frames.remove(frame);
            }
        }
    }

    /// Writes `frame`'s sheet as the refresh of `order` read it, and
    /// answers the rules it holds; none for a document no rule applies to,
    /// such as `about:blank`.
    async fn write(
        &self,
        frame: &page::FrameId,
        lifted: &BTreeSet<String>,
        order: u64,
    ) -> Result<Option<Vec<CssRule>>, Failure> {
        let Some(read) = self
            .driver
            .read_in_frame(frame, COLLECT, Value::Null)
            .await?
        else {
            return Ok(None);
        };
        let Ok(read) = Collected::deserialize(&read) else {
            return Ok(None);
        };
        if !(read.url.starts_with("http://") || read.url.starts_with("https://")) {
            return Ok(None);
        }
        let classes: Vec<&str> = read.classes.iter().map(String::as_str).collect();
        let ids: Vec<&str> = read.ids.iter().map(String::as_str).collect();
        let rules = PageRules::of(&self.cosmetic.lists, &self.cosmetic.choices, &read.url)
            .css_rules(&classes, &ids);
        self.sheets
            .set(
                (self.driver.session(), frame),
                Part::Hiding,
                hiding::sheet(&rules, lifted),
                Some(order),
            )
            .await?;
        Ok(Some(rules))
    }

    /// The cosmetic rules the sheet of the page's main frame applies now,
    /// but for those lifted for its Session.
    async fn applied_rules(&self) -> Vec<CssRule> {
        let lifted = self.session.lifted.lock().await.clone();
        self.frames
            .lock()
            .await
            .get(self.driver.main_frame())
            .into_iter()
            .flat_map(|(_, rules)| rules)
            .filter(|rule| !lifted.contains(&rule.selector))
            .cloned()
            .collect()
    }

    /// The selectors of the cosmetic rules the sheet of the page's main
    /// frame applies now, but for those lifted for its Session.
    pub(crate) async fn applied(&self) -> Vec<String> {
        self.applied_rules()
            .await
            .into_iter()
            .map(|rule| rule.selector)
            .collect()
    }

    /// Lifts the rules of `applied` whose selectors are in `hiding` for the
    /// page's Session, writes every sheet of the Session's pages again
    /// without them and records each in Provenance, for the action
    /// `operation` on the target `selector`.
    async fn lift_rules(
        &self,
        applied: &[CssRule],
        hiding: &[String],
        (selector, operation): (&str, &str),
    ) {
        self.session
            .lifted
            .lock()
            .await
            .extend(hiding.iter().cloned());
        let pages: Vec<Arc<PageHiding>> = self
            .session
            .pages
            .lock()
            .await
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        for page in pages {
            let frames: Vec<page::FrameId> = page.frames.lock().await.keys().cloned().collect();
            for frame in frames {
                page.refresh(&frame).await;
            }
        }
        for rule in applied
            .iter()
            .filter(|rule| hiding.contains(&rule.selector))
        {
            trace::provenance(
                &self.run,
                &json!({
                    "action": "live_hiding_lifted",
                    "page_id": self.page_id,
                    "session_id": self.session_id,
                    "rule": rule.selector,
                    "list": rule.list.name(),
                    "operation": operation,
                    "target": selector,
                }),
            )
            .await;
        }
    }
}

/// What hides elements on one page, which the guard lifts from an action's
/// target (ADRs 0047 and 0048): its live hiding, when it hides anything,
/// and the Consent prompt runner's rules in its documents' sheets.
#[derive(Clone, Copy)]
pub(crate) struct Hidden<'a> {
    /// The page's live hiding, when it hides anything.
    pub(crate) live: Option<&'a PageHiding>,
    /// Its documents' sheets, which the runner writes into.
    pub(crate) sheets: &'a Sheets,
    /// Its Run, whose Provenance records each lift.
    pub(crate) run: &'a Run,
    /// The page.
    pub(crate) page_id: &'a PageId,
    /// Its Session.
    pub(crate) session_id: &'a SessionId,
}

impl Hidden<'_> {
    /// When a rule hid the target `selector` matches on the page's main
    /// frame, lifts it: a live-hiding rule for the page's Session, every
    /// sheet of the Session's pages written again without it, and a rule of
    /// the Consent prompt runner from the main document's sheet until its
    /// next load; Provenance records each, for the action `operation`.
    /// Answers whether it lifted any, so that the guard looks again at once.
    pub(crate) async fn lift(
        &self,
        turn: &mut Turn<'_>,
        selector: &str,
        operation: &str,
    ) -> Result<bool, Unanswered> {
        let live = match self.live {
            Some(page) => page.applied_rules().await,
            None => Vec::new(),
        };
        let runner = self.sheets.runner_applied();
        if live.is_empty() && runner.is_empty() {
            return Ok(false);
        }
        let rules: Vec<&str> = live
            .iter()
            .map(|rule| rule.selector.as_str())
            .chain(runner.iter().map(String::as_str))
            .collect();
        let answered = turn
            .call(HIDDEN_BY, json!({ "selector": selector, "rules": rules }))
            .await?;
        let hiding: Vec<String> = Vec::deserialize(&answered).unwrap_or_default();
        if hiding.is_empty() {
            return Ok(false);
        }
        if let Some(page) = self.live
            && live.iter().any(|rule| hiding.contains(&rule.selector))
        {
            page.lift_rules(&live, &hiding, (selector, operation)).await;
        }
        let runner: Vec<String> = runner
            .into_iter()
            .filter(|rule| hiding.contains(rule))
            .collect();
        if !runner.is_empty() {
            if let Err(failure) = self.sheets.lift_runner(runner.clone()).await {
                tracing::debug!(%failure, "the Consent prompt runner's lifted rules were not written: its document went");
            }
            for rule in &runner {
                trace::provenance(
                    self.run,
                    &json!({
                        "action": "live_hiding_lifted",
                        "page_id": self.page_id,
                        "session_id": self.session_id,
                        "rule": rule,
                        // A prompt the Consent prompt runner hid names no list.
                        "list": "consent",
                        "operation": operation,
                        "target": selector,
                    }),
                )
                .await;
            }
        }
        Ok(true)
    }
}
