//! The Consent prompt runner's engine side on Chromium (ADR 0048; spec
//! #577's `consent`): autoconsent's vendor rules, then its `reject`
//! heuristics, in every frame, out-of-process iframes included, then hiding
//! a prompt with no refusal that leaves the content readable. The runner's
//! bundle (the Clean-up crate's [`Runner`]) runs in the guard's isolated
//! world of every frame from each new document; the engine starts it there
//! once it added the world's binding, `Runtime.addBinding` named by the
//! world's name, again for every new document since with `Runtime` off V8
//! carries no binding to a new context, and reads its reports from
//! `Runtime.bindingCalled`, so nothing polls and `Runtime` is never
//! enabled. For each report the engine:
//!
//! - clicks the element the runner handed the guard, with the guard's
//!   check and arm, trusted input at the point in the page's viewport, and
//!   its disarm, holding the page's pointer so no locator action's gesture
//!   comes between; for a frame's element, every other frame's hiding is
//!   lifted for the click, so a hidden owner never swallows it, and the
//!   press waits until the pointer, moved there, reaches the element in its
//!   frame, since the browser sends another process's frame its input by
//!   where the page last drew it, which the lifting changes only once the
//!   page drew again;
//! - runs a snippet the snippet table classes in the main world there,
//!   through `DOM.resolveNode`, which resolves into the frame's main world,
//!   and `Runtime.callFunctionOn`;
//! - sets its part of the document's hiding sheet, the one live hiding
//!   writes too ([`Sheets`]), which no `document.styleSheets` lists;
//! - answers whether a consent wall is accepted: only on a domain a person
//!   approved in `consent.domains`, and never while it only detects;
//! - records what it did with each prompt in the Run trace's Provenance,
//!   an acceptance with its domain and the approval it came from, and
//!   keeps a wall left in the page's main frame for the page's next call,
//!   which raises it as `blocked` with `consent_required` (`walls`), and
//!   records its domain in the Run index, which "Needs you" offers for
//!   `consent.domains`.
//!
//! With `consent.handling: detect` the runner still starts, to run the wall
//! test, but only looks: it clicks nothing, hides nothing and runs nothing
//! in the main world. With `consent.handling: off` nothing of the runner is
//! registered or started, and no wall is reported (#2434). A prompt with no refusal that leaves the content
//! readable and asks for no payment is hidden while the runner answers;
//! any other in the top frame is a consent wall. The Decision
//! provider (area 9's) is not here yet, nor `consent.on_wall: ask`, which
//! needs the Human pause to take a person's answer (A05-38) and opens no
//! page meanwhile. Firefox and `WebKit`'s route is A05-45's.

mod frames;
mod messages;
mod walls;

use std::sync::Arc;
use std::time::Duration;

use goodfetch_cdp::SessionId as TargetSession;
use goodfetch_cdp::Subscription;
use goodfetch_cdp::generated::{dom, page, runtime, target};
use goodfetch_clean_up_rules::consent::{self as bundle, Runner, World};
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::runs::{Run, identity};
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CONSENT_DOMAINS, CONSENT_ON_WALL, CapabilityUnavailableDetails,
};
use serde_json::{Value, json};
use tokio::sync::{Mutex, OnceCell};

use self::frames::{Frame, Frames};
use self::messages::{ANSWER, Action, BINDING, DETECTING, Message, Outcome, RESTART, START};
pub(crate) use self::walls::Walls;
use self::walls::{Approval, Wall};
use crate::driver::{Child, ChildKind, GUARD_WORLD, OnChild, PageDriver};
use crate::gesture_order::PointerAsks;
use crate::guard::{self, Element, Readiness};
use crate::hiding::{Part, Sheets};
use crate::interception::{ConsentHandling, domain_key};
use crate::trace;

/// How long the runner's click may wait for its target to be ready.
const CLICK_WINDOW: Duration = Duration::from_secs(3);

/// The longest one check of a click's target waits in the page.
const SLICE: Duration = Duration::from_millis(500);

/// How long past its own wait a frame's guard world may take to answer
/// before the frame counts as gone.
const GRACE: Duration = Duration::from_secs(2);

/// The longest a frame's guard world waits for one move of the pointer to
/// reach a click's target before the engine moves it again.
const MOVE_WAIT: Duration = Duration::from_millis(100);

/// The watch, in a frame's guard world, of whether the pointer reaches a
/// click's target there.
const POINTER: &str = include_str!("consent/pointer.js");

/// The engine's runner bundle, decompressed on the first page that needs
/// it, through a blocking task of that page's Run (ADR 0060), then shared;
/// and the consent walls each page's runner left for its next call.
#[derive(Clone, Default)]
pub(crate) struct Consent {
    runner: Arc<OnceCell<Arc<Runner>>>,
    walls: Walls,
}

impl Consent {
    /// The consent walls the pages' runners left.
    pub(crate) fn walls(&self) -> &Walls {
        &self.walls
    }

    /// Starts the Consent prompt runner on the page `page_id`, which
    /// `driver` drives for `run`: the bundle in the guard's world of every
    /// frame from the next document on, out-of-process iframes attached as
    /// they appear, hiding through the page's `sheets`; with the Run's
    /// `consent.handling: detect`, only looking; with `off`, not at all
    /// (#2434). It answers what the runner does with each of the page's
    /// children before it runs: an out-of-process iframe is prepared as the
    /// page is, and with `off` none is.
    ///
    /// # Errors
    ///
    /// `capability_unavailable` for a Run whose `consent.on_wall` is
    /// `ask` and whose `consent.handling` is not `off`, until the Human pause takes a person's answer; the code of
    /// opening a page when the browser refuses a command.
    pub(crate) async fn start(
        &self,
        page_id: &PageId,
        (driver, pointer): (&PageDriver, PointerAsks),
        run: &Run,
        sheets: &Arc<Sheets>,
    ) -> Result<OnChild, CallError> {
        let answering = match ConsentHandling::of(identity(run)) {
            ConsentHandling::Answer => true,
            ConsentHandling::Detect => false,
            // Nothing judges a wall, so `consent.on_wall` asks nothing either.
            ConsentHandling::Off => return Ok(Arc::new(|_: &Child| Box::pin(async { true }))),
        };
        let resolved = run
            .settings()
            .map(goodfetch_engine::settings::RunSettings::resolved);
        if let Some(asked) = resolved
            .and_then(|resolved| resolved.effective(&CONSENT_ON_WALL))
            .filter(|effective| effective.value == json!("ask"))
        {
            return Err(CallError::new(
                &CAPABILITY_UNAVAILABLE,
                format!(
                    "`consent.on_wall` is `ask` in {}, but this engine cannot yet pause for a person's answer to a consent wall: set it to `report`, which raises `blocked` with `consent_required`.",
                    asked.layer.as_str()
                ),
                &CapabilityUnavailableDetails {
                    key: CONSENT_ON_WALL.name.to_owned(),
                    layer: asked.layer,
                    kind: "ask".to_owned(),
                    fix: "`consent.on_wall: report`".to_owned(),
                },
            ));
        }
        let approval = resolved
            .and_then(|resolved| resolved.effective(&CONSENT_DOMAINS))
            .and_then(|effective| Approval::of(&effective.value, effective.layer));
        let runner = self
            .runner
            .get_or_init(|| async {
                let loaded = run.owner().blocking(Runner::embedded).await.expect(
                    "invariant: the build wrote the Consent prompt runner's bundle with this binary",
                );
                Arc::new(loaded)
            })
            .await;
        let shared = Arc::new(Shared {
            driver: driver.clone(),
            pointer,
            run: run.clone(),
            clock: run.clock(),
            runner: Arc::clone(runner),
            frames: Mutex::new(Frames::new(
                driver.session().clone(),
                driver.main_frame().clone(),
            )),
            sheets: Arc::clone(sheets),
            page_id: page_id.clone(),
            answering,
            approval,
            walls: self.walls.clone(),
        });
        let session = driver.session().clone();
        let events = driver.events_on(&session);
        prepare(&shared, driver).await?;
        run.spawn(listen(Arc::clone(&shared), session, events));
        Ok(Arc::new(move |child: &Child| {
            let frame = (child.kind == ChildKind::Frame).then(|| {
                (
                    Arc::clone(&shared),
                    child.parent.clone(),
                    child.session.clone(),
                    child.target.clone(),
                )
            });
            Box::pin(async move {
                if let Some((shared, parent, session, target)) = frame {
                    attach(shared, parent, session, target).await;
                }
                // Prepared or not, the frame runs, as the runner only looks.
                true
            })
        }))
    }
}

/// Whether the runner's first pass on the main document of the page `driver`
/// drives found a prompt, ended or not, as the runner answers in its world:
/// a vendor's, a pre-hide rule's or one the heuristics know, or one the page
/// shows now that its pass would find. The read is one of its navigation's
/// detection reads, counted on `work` (ADR 0140's C4).
pub(crate) async fn first_pass_found(driver: &PageDriver, work: &CountedWork) -> bool {
    work.count(Row::NavigationDetectionReads);
    matches!(
        driver
            .read_in_frame(driver.main_frame(), DETECTING, Value::Null)
            .await,
        Ok(Some(Value::String(stands))) if stands != "none"
    )
}

/// One page's runner: its driver, its frames, its hiding sheets, whether
/// it answers, who approved accepting a wall, and the walls it leaves.
struct Shared {
    driver: PageDriver,
    /// Where it asks for the page's pointer, which each click holds.
    pointer: PointerAsks,
    run: Run,
    clock: Clock,
    runner: Arc<Runner>,
    frames: Mutex<Frames>,
    sheets: Arc<Sheets>,
    page_id: PageId,
    answering: bool,
    approval: Option<Approval>,
    walls: Walls,
}

/// Puts the bundle on every new document of the target `driver` sends its
/// commands to, the page's or an out-of-process iframe's, a failure
/// translated as opening a page's (`errors`); the driver attaches the
/// iframes, each paused until it is prepared too (`driver::follow_children`).
async fn prepare(shared: &Shared, driver: &PageDriver) -> Result<(), CallError> {
    driver
        .opening_command(&page::AddScriptToEvaluateOnNewDocumentParams {
            source: shared.runner.source().to_owned(),
            world_name: Some(GUARD_WORLD.to_owned()),
            include_command_line_api: None,
            run_immediately: Some(true),
        })
        .await?;
    Ok(())
}

/// Reads `session`'s events until its connection ends, handing each one
/// that asks something to a task of its own, so the connection never waits
/// on this page.
async fn listen(shared: Arc<Shared>, session: TargetSession, mut events: Subscription) {
    while let Some(event) = events.next().await {
        react(&shared, &session, &event).await;
    }
    // The page's own target is gone: so is any wall it left.
    if session == *shared.driver.session() {
        shared.walls.new_document(&shared.page_id);
    }
}

/// What `event` of `session` asks of the runner.
async fn react(shared: &Arc<Shared>, session: &TargetSession, event: &goodfetch_cdp::RawEvent) {
    if let Some(detached) = event.read::<target::DetachedFromTargetEvent>() {
        let detached = TargetSession(detached.session_id);
        shared.frames.lock().await.forget_session(&detached);
        shared.sheets.forget_session(&detached);
    } else if let Some(navigated) = event.read::<page::FrameNavigatedEvent>() {
        shared
            .sheets
            .navigated(session, &navigated.frame.id, &navigated.frame.loader_id);
        // A new document in the page's main frame leaves the last one's
        // wall behind.
        if *session == *shared.driver.session() && navigated.frame.parent_id.is_none() {
            shared.walls.new_document(&shared.page_id);
        }
        let shared = Arc::clone(shared);
        let session = session.clone();
        shared.run.clone().spawn(async move {
            let frame = navigated.frame;
            begin(&shared, &session, frame.id, frame.loader_id).await;
        });
    } else if let Some(moved) = event.read::<page::NavigatedWithinDocumentEvent>() {
        let context = shared.frames.lock().await.context(session, &moved.frame_id);
        if let Some(context) = context {
            let shared = Arc::clone(shared);
            let session = session.clone();
            shared.run.clone().spawn(async move {
                let _ = shared
                    .driver
                    .call_in(&session, context, RESTART, Value::Null)
                    .await;
            });
        }
    } else if let Some(detached) = event.read::<page::FrameDetachedEvent>() {
        shared
            .frames
            .lock()
            .await
            .forget_frame(session, &detached.frame_id);
        shared.sheets.forget_frame(session, &detached.frame_id);
    } else if let Some(called) = event.read::<runtime::BindingCalledEvent>()
        && called.name == BINDING
    {
        let frame = shared
            .frames
            .lock()
            .await
            .frame_of(session, called.execution_context_id);
        let Some(frame) = frame else { return };
        let Some(message) = Message::read(&called.payload) else {
            tracing::debug!("the Consent prompt runner sent a message the engine does not read");
            return;
        };
        // What the first pass found, and a wall left, are taken in the
        // order the runner reported them, before the page's next event, so
        // a call waiting on the pass's end finds its wall.
        match &message {
            Message::Detected | Message::Settled => {
                first_pass(shared, &frame, &message).await;
                return;
            }
            Message::Outcome(outcome) if outcome.action == Action::Wall => {
                left_wall(shared, &frame, outcome).await;
            }
            _ => {}
        }
        let shared = Arc::clone(shared);
        shared.run.clone().spawn(async move {
            answer(&shared, &frame, message).await;
        });
    }
}

/// Prepares the out-of-process iframe on `session`, whose target is
/// `target`, a child of `parent`, before it runs.
async fn attach(
    shared: Arc<Shared>,
    parent: TargetSession,
    session: TargetSession,
    target: target::TargetID,
) {
    shared.frames.lock().await.attach(&parent, &session, target);
    let events = shared.driver.events_on(&session);
    let iframe = shared.driver.on_child(session.clone());
    let prepared = async {
        iframe
            .opening_command(&page::EnableParams::default())
            .await?;
        prepare(&shared, &iframe).await
    }
    .await;
    match prepared {
        Ok(()) => shared.run.clone().spawn(listen(shared, session, events)),
        Err(failure) => {
            tracing::debug!(%failure, "an out-of-process iframe was not prepared for the Consent prompt runner");
        }
    }
}

/// Starts the runner in the guard's world of `frame`'s new document on
/// `session`, once its binding is there.
async fn begin(
    shared: &Shared,
    session: &TargetSession,
    frame: page::FrameId,
    loader: goodfetch_cdp::generated::network::LoaderId,
) {
    let driver = &shared.driver;
    let made = driver
        .command_on(
            session,
            &page::CreateIsolatedWorldParams {
                frame_id: frame.clone(),
                world_name: Some(GUARD_WORLD.to_owned()),
                grant_univeral_access: None,
                content_security_policy: None,
            },
        )
        .await;
    let Ok(made) = made else { return };
    let context = made.execution_context_id;
    shared
        .frames
        .lock()
        .await
        .document(session, frame, context, loader);
    let bound = driver
        .command_on(
            session,
            &runtime::AddBindingParams {
                name: BINDING.to_owned(),
                execution_context_id: None,
                execution_context_name: Some(GUARD_WORLD.to_owned()),
            },
        )
        .await;
    if bound.is_err() {
        return;
    }
    let options = json!({ "binding": BINDING, "answering": shared.answering });
    // The runner's start detects in the document from its first script on,
    // apart from the read that judges it once its navigation ended (ADR
    // 0140, open question 3).
    shared
        .run
        .metrics()
        .counted_work()
        .count(Row::NavigationDetectionReads);
    if let Ok(Value::String(started)) = driver.call_in(session, context, START, options).await
        && started == "absent"
    {
        tracing::debug!("a document's guard world never ran the Consent prompt runner's bundle");
    }
}

/// Carries out `message` of the runner in `frame`, and answers it.
async fn answer(shared: &Shared, frame: &Frame, message: Message) {
    let (id, value) = match message {
        Message::Click { id, handle } => (id, json!(click(shared, frame, handle).await)),
        Message::Eval { id, snippet, code } => {
            let ran = bundle::world(&snippet) == Some(World::Main)
                && main_world(shared, frame, &code).await.unwrap_or(false);
            (id, json!(ran))
        }
        Message::Sheet { id, text } => {
            // A sheet for a document that went is never written into the
            // frame's next one.
            if current(shared, frame).await
                && let Err(failure) = shared
                    .sheets
                    .set((&frame.session, &frame.id), Part::Consent, text, None)
                    .await
            {
                tracing::debug!(%failure, "the Consent prompt runner's hiding sheet was not set");
            }
            (id, json!(true))
        }
        Message::Wall { id, .. } => (id, json!(wall_answer(shared, frame).await)),
        Message::Outcome(outcome) => {
            tracing::debug!(action = ?outcome.action, rule = %outcome.rule, "Consent prompt");
            recorded(shared, frame, &outcome).await;
            return;
        }
        Message::Detected | Message::Settled => return,
    };
    let _ = shared
        .driver
        .call_in(
            &frame.session,
            frame.context,
            ANSWER,
            json!({ "id": id, "value": value }),
        )
        .await;
}

/// The engine's answer to a consent wall in `frame`'s document: `accept`
/// only while the runner answers and on a domain `consent.domains`
/// approves, `block` otherwise.
async fn wall_answer(shared: &Shared, frame: &Frame) -> &'static str {
    let approved = shared.answering
        && match &shared.approval {
            Some(approval) => document_url(shared, frame)
                .await
                .is_some_and(|url| approval.covers(&url).is_some()),
            None => false,
        };
    if approved { "accept" } else { "block" }
}

/// The URL of `frame`'s current document, read in its guard world.
async fn document_url(shared: &Shared, frame: &Frame) -> Option<String> {
    match shared
        .driver
        .call_in(
            &frame.session,
            frame.context,
            "function url() { return location.href; }",
            Value::Null,
        )
        .await
    {
        Ok(Value::String(url)) => Some(url),
        _ => None,
    }
}

/// Records `outcome`, what the runner did with a prompt of `frame`'s
/// document, in the Run trace's Provenance: an acceptance with its domain
/// and the approval it came from. A wall left in the page's main frame's
/// current document is kept for the page's next call.
async fn recorded(shared: &Shared, frame: &Frame, outcome: &Outcome) {
    let mut entry = json!({
        "action": "consent",
        "page_id": shared.page_id,
        "url": outcome.url,
        "answer": outcome.action,
        "rule": outcome.rule,
    });
    if let Some(verified) = outcome.verified {
        entry["verified"] = json!(verified);
    }
    if let Some(reason) = outcome.reason {
        entry["reason"] = json!(reason);
    }
    match outcome.action {
        Action::Accepted => {
            if let Some(approval) = &shared.approval {
                entry["domain"] = json!(approval.covers(&outcome.url));
                entry["approval"] = json!({ "key": CONSENT_DOMAINS.name, "layer": approval.layer });
            }
        }
        Action::Wall => {
            let rule = outcome
                .reason
                .unwrap_or(messages::WallReason::Covers)
                .rule();
            entry["cause"] = json!(goodfetch_protocol::block::BlockCause::ConsentRequired);
            entry["block_rule"] = json!(rule);
            // "Needs you" offers the domain for `consent.domains`.
            if main(shared, frame)
                && let Some(domain) = domain_key(&outcome.url)
                && let Err(error) = shared.run.record_consent_wall(domain).await
            {
                tracing::warn!(%error, "the Run index did not record the consent wall: \"Needs you\" does not offer its domain");
            }
        }
        Action::Refused | Action::Hidden | Action::Left | Action::Failed => {}
    }
    trace::counted_provenance(&shared.run, Row::NavigationProvenanceEntries, entry).await;
}

/// Whether `frame` is the page's main frame.
fn main(shared: &Shared, frame: &Frame) -> bool {
    frame.session == *shared.driver.session() && frame.id == *shared.driver.main_frame()
}

/// Whether `frame`'s document is its frame's current one.
async fn current(shared: &Shared, frame: &Frame) -> bool {
    shared
        .frames
        .lock()
        .await
        .context(&frame.session, &frame.id)
        == Some(frame.context)
}

/// Keeps the wall `outcome` left in `frame`'s document for the page's next
/// call, when that document is the page's main frame's current one.
async fn left_wall(shared: &Shared, frame: &Frame, outcome: &Outcome) {
    let rule = outcome
        .reason
        .unwrap_or(messages::WallReason::Covers)
        .rule();
    let frames = shared.frames.lock().await;
    let current = frames.context(&frame.session, &frame.id) == Some(frame.context);
    let loader = frames.loader(&frame.session, &frame.id);
    drop(frames);
    if main(shared, frame)
        && current
        && let Some(loader) = loader
    {
        shared
            .walls
            .put(shared.page_id.clone(), Wall { loader, rule });
    }
}

/// Takes what the runner's first pass on `frame`'s document reported,
/// `message`: in the page's main frame, a prompt found makes the page's
/// calls wait for the pass's end; the pass's end lets them go, and starts
/// live hiding on the document, which waited for it (ADR 0048).
async fn first_pass(shared: &Shared, frame: &Frame, message: &Message) {
    if !current(shared, frame).await {
        return;
    }
    let main = main(shared, frame);
    match message {
        Message::Detected if main => shared.walls.detected(&shared.page_id),
        Message::Settled => {
            if main {
                shared.walls.settled(&shared.page_id);
            }
            if let Err(failure) = shared.sheets.release(&frame.session, &frame.id, None).await {
                tracing::debug!(%failure, "live hiding did not start on a document after the Consent prompt runner's first pass");
            }
        }
        _ => {}
    }
}

/// Clicks the element `handle` of `frame`'s guard world through the guard,
/// within [`CLICK_WINDOW`]; whether it landed on it.
async fn click(shared: &Shared, frame: &Frame, handle: u64) -> bool {
    let driver = &shared.driver;
    let element = Element {
        session: &frame.session,
        context: frame.context,
        handle,
    };
    let end = shared.clock.now() + CLICK_WINDOW;
    loop {
        let remaining = end.saturating_duration_since(shared.clock.now());
        if remaining.is_zero() {
            return false;
        }
        let Some(pointer) = shared.pointer.lent().await else {
            return false;
        };
        let slice = remaining.min(SLICE);
        // A frame that goes away with its document may never answer: the
        // pointer is not held past the check's own slice and some grace.
        let checked =
            tokio::time::timeout(slice + GRACE, guard::check_element(driver, &element, slice))
                .await
                .unwrap_or(Readiness::Gone);
        match checked {
            Readiness::Ready { x, y } => {
                let Some(chain) = shared.frames.lock().await.owner_chain(frame) else {
                    return false;
                };
                let Ok((dx, dy)) = offset(shared, &chain).await else {
                    return false;
                };
                shared.sheets.lift(&frame.session, &frame.id).await;
                let reached =
                    chain.is_empty() || reach(shared, &element, (x + dx, y + dy), end).await;
                let pressed = if reached {
                    driver.press(x + dx, y + dy).await
                } else {
                    Ok(())
                };
                // A frame gone with the click never answers the disarm: the
                // click landed, as when its document went away.
                let landed = reached
                    && pressed.is_ok()
                    && tokio::time::timeout(GRACE, guard::landed_on(driver, &element))
                        .await
                        .unwrap_or(true);
                shared.sheets.restore().await;
                drop(pointer);
                if landed {
                    return true;
                }
            }
            Readiness::Waiting => {}
            Readiness::Gone => return false,
        }
    }
}

/// Moves the pointer to (`x`, `y`) of the page's viewport until a move reaches
/// `element` in its frame, within `end`; whether one did. The browser routes
/// each move as it would route a press, so a press that follows one goes
/// where it went.
async fn reach(
    shared: &Shared,
    element: &Element<'_>,
    (x, y): (f64, f64),
    end: tokio::time::Instant,
) -> bool {
    let driver = &shared.driver;
    let watch = |wait: Duration| {
        let argument = json!({
            "handle": element.handle,
            "wait": u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
        });
        tokio::time::timeout(
            wait + GRACE,
            driver.call_in(element.session, element.context, POINTER, argument),
        )
    };
    if !matches!(watch(Duration::ZERO).await, Ok(Ok(Value::String(seen))) if seen != "gone") {
        return false;
    }
    loop {
        let remaining = end.saturating_duration_since(shared.clock.now());
        if remaining.is_zero() || driver.move_pointer(x, y).await.is_err() {
            return false;
        }
        match watch(remaining.min(MOVE_WAIT)).await {
            Ok(Ok(Value::String(seen))) if seen == "over" => return true,
            Ok(Ok(Value::String(seen))) if seen != "gone" => {}
            _ => return false,
        }
    }
}

/// The distance of `frame`'s viewport from the page's: the content box of
/// each owner element on the way up, `chain`, read on its parent's target,
/// each owner scrolled into its parent's view first.
async fn offset(
    shared: &Shared,
    chain: &[(TargetSession, page::FrameId)],
) -> Result<(f64, f64), goodfetch_cdp::Failure> {
    let driver = &shared.driver;
    let (mut x, mut y) = (0.0, 0.0);
    for (session, frame) in chain {
        let owner = driver
            .command_on(
                session,
                &dom::GetFrameOwnerParams {
                    frame_id: frame.clone(),
                },
            )
            .await?;
        let _ = driver
            .command_on(
                session,
                &dom::ScrollIntoViewIfNeededParams {
                    node_id: None,
                    backend_node_id: Some(owner.backend_node_id),
                    object_id: None,
                    rect: None,
                },
            )
            .await;
        let model = driver
            .command_on(
                session,
                &dom::GetBoxModelParams {
                    node_id: None,
                    backend_node_id: Some(owner.backend_node_id),
                    object_id: None,
                },
            )
            .await?
            .model;
        x += model.content.first().copied().unwrap_or_default();
        y += model.content.get(1).copied().unwrap_or_default();
    }
    Ok((x, y))
}

/// Runs `code` in `frame`'s main world with `Runtime` off: the guard's
/// world's `document` resolved, by its node, into the main world, where
/// `Runtime.callFunctionOn` calls it; its result as a boolean.
async fn main_world(
    shared: &Shared,
    frame: &Frame,
    code: &str,
) -> Result<bool, goodfetch_cdp::Failure> {
    let driver = &shared.driver;
    let document = driver
        .command_on(
            &frame.session,
            &runtime::EvaluateParams {
                expression: "document".to_owned(),
                object_group: None,
                include_command_line_api: None,
                silent: Some(true),
                context_id: Some(frame.context),
                return_by_value: Some(false),
                generate_preview: None,
                user_gesture: None,
                await_promise: None,
                throw_on_side_effect: None,
                timeout: None,
                disable_breaks: None,
                repl_mode: None,
                allow_unsafe_eval_blocked_by_csp: None,
                unique_context_id: None,
                serialization_options: None,
            },
        )
        .await?;
    let Some(isolated) = document.result.object_id else {
        return Ok(false);
    };
    let node = driver
        .command_on(
            &frame.session,
            &dom::DescribeNodeParams {
                node_id: None,
                backend_node_id: None,
                object_id: Some(isolated.clone()),
                depth: None,
                pierce: None,
            },
        )
        .await;
    release(shared, frame, isolated).await;
    let resolved = driver
        .command_on(
            &frame.session,
            &dom::ResolveNodeParams {
                node_id: None,
                backend_node_id: Some(node?.node.backend_node_id),
                object_group: None,
                execution_context_id: None,
            },
        )
        .await?;
    let Some(main) = resolved.object.object_id else {
        return Ok(false);
    };
    let called = driver
        .command_on(
            &frame.session,
            &runtime::CallFunctionOnParams {
                function_declaration: format!("function () {{ return Boolean({code}); }}"),
                object_id: Some(main.clone()),
                arguments: None,
                silent: Some(true),
                return_by_value: Some(true),
                generate_preview: None,
                user_gesture: None,
                await_promise: Some(true),
                execution_context_id: None,
                object_group: None,
                throw_on_side_effect: None,
                unique_context_id: None,
                serialization_options: None,
            },
        )
        .await;
    release(shared, frame, main).await;
    let called = called?;
    Ok(called.exception_details.is_none() && called.result.value == Some(Value::Bool(true)))
}

/// Lets the browser forget `object`.
async fn release(shared: &Shared, frame: &Frame, object: runtime::RemoteObjectId) {
    let _ = shared
        .driver
        .command_on(
            &frame.session,
            &runtime::ReleaseObjectParams { object_id: object },
        )
        .await;
}
