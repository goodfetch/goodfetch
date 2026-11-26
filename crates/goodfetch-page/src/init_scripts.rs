//! A page's init scripts (`page.add_init_script`, ADR 0085), which its owner
//! holds (ADR 0146): each is added on the page and on its out-of-process
//! frames, and removed once its call is dropped or once the page's request
//! part, which it belongs to, is replaced. A navigation or a Gesture sent
//! after an init script's call waits until the script is registered, or
//! removed once its call was cancelled ([`InitScripts::settling`]).
#![expect(
    clippy::disallowed_types,
    reason = "the lock guards one page's live scripts and is never held across an `.await`, so a replacement of the page's request part takes its scripts out at once; the Page ID it reads under it is the only other lock it is held with, always in that order, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_util::future::join_all;
use goodfetch_engine::CallError;
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::page::functions::PAGE_ADD_INIT_SCRIPT;
use tokio::sync::{oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::cdp::{CdpPage, InitScriptId};
use crate::page_children::{Child, ChildKind, OnChild};

/// Locks `mutex`, whose holders never panic while they hold it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One page's init scripts, as its calls add and remove them, its
/// out-of-process frames take them and its navigations wait for them.
/// Clones share them.
#[derive(Clone)]
pub(crate) struct InitScripts {
    shared: Arc<Shared>,
}

struct Shared {
    /// The primitives the page takes scripts by.
    cdp: CdpPage,
    /// Whose tasks add and remove them.
    run: RunIdentity,
    /// How many are on their way in or out: a navigation waits for none to
    /// be left, so that one sent after `page.add_init_script` runs the
    /// script, and one sent after its call was cancelled runs without it.
    pending: watch::Sender<usize>,
    /// The Page ID of the request the page serves now, whose calls' scripts
    /// are live (ADR 0146); written under `frames`.
    page_id: watch::Sender<PageId>,
    /// The browser runs a script added on the page's own session in the
    /// frames of the page's process only, so each live script is also added
    /// on each out-of-process frame's session, before the frame runs or,
    /// for one already running, for its next document.
    frames: Mutex<Frames>,
}

#[derive(Default)]
struct Frames {
    /// The key the next script takes.
    next: u64,
    /// Each live script, by its key.
    live: BTreeMap<u64, Live>,
    /// The page's out-of-process frames attached, each until its target
    /// goes.
    iframes: Vec<(CdpPage, CancellationToken)>,
}

/// A live script: its ID on the page's own session, its source, and where
/// each iframe holds it.
struct Live {
    script: InitScriptId,
    source: String,
    held: Vec<(CdpPage, InitScriptId)>,
}

impl InitScripts {
    /// The init scripts of the page `cdp` reaches, serving the request
    /// `page_id`, none yet, added and removed by tasks of `run`.
    pub(crate) fn new(cdp: CdpPage, run: RunIdentity, page_id: PageId) -> Self {
        Self {
            shared: Arc::new(Shared {
                cdp,
                run,
                pending: watch::Sender::new(0),
                page_id: watch::Sender::new(page_id),
                frames: Mutex::default(),
            }),
        }
    }

    /// What the init scripts do with each of the page's out-of-process
    /// frames, which `page_children::follow` hands over: each live script
    /// is added on it before it runs.
    pub(crate) fn frames(&self) -> OnChild {
        let scripts = self.clone();
        Arc::new(move |child: &Child| {
            let frame = (child.kind == ChildKind::Frame).then(|| {
                (
                    scripts.clone(),
                    scripts.shared.cdp.frame(&child.session),
                    child.gone.clone(),
                )
            });
            Box::pin(async move {
                if let Some((scripts, cdp, gone)) = frame {
                    scripts.attached(cdp, gone).await;
                }
                true
            })
        })
    }

    /// Counts one script on its way, until the answer is dropped.
    pub(crate) fn begin(&self) -> Pending {
        self.shared.pending.send_modify(|pending| *pending += 1);
        Pending {
            scripts: self.clone(),
        }
    }

    /// The wait until no script is on its way in or out; none while none
    /// is.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the wait.
    pub(crate) fn settling(&self) -> Option<impl Future<Output = ()> + Send + use<>> {
        let mut pending = self.shared.pending.subscribe();
        let on_their_way = *pending.borrow_and_update() > 0;
        on_their_way.then_some(async move {
            let _ = pending.wait_for(|pending| *pending == 0).await;
        })
    }

    /// Registers the script `source` for the request `page_id`, `pending`
    /// until it is: on a task of the Run's, so a call dropped meanwhile
    /// never leaves the script registered, as its registration, answered
    /// to no one, is removed there, counted before `pending` goes; and one
    /// whose request was replaced meanwhile is removed there too. Never
    /// answers when the Run ends under it.
    ///
    /// # Errors
    ///
    /// The code of `page.add_init_script` when the page cannot take it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future removes the script.
    pub(crate) fn register(
        &self,
        source: String,
        pending: Pending,
        page_id: &PageId,
    ) -> impl Future<Output = Result<Registered, CallError>> + Send + use<> {
        let (answer, answered) = oneshot::channel::<Result<Registered, CallError>>();
        let (scripts, page_id) = (self.clone(), page_id.clone());
        self.shared.run.spawn(async move {
            let registered = async {
                let script = scripts
                    .shared
                    .cdp
                    .add_init_script(source.clone(), PAGE_ADD_INIT_SCRIPT.name)
                    .await?;
                // The page's out-of-process iframes take it too.
                let key = scripts.add(script, source, &page_id).await;
                Ok::<_, CallError>(Registered {
                    scripts,
                    key,
                    page_id,
                })
            }
            .await;
            drop(answer.send(registered));
            drop(pending);
        });
        async move {
            match answered.await {
                Ok(registered) => registered,
                // The Run ended under it: its ending answers.
                Err(_) => std::future::pending().await,
            }
        }
    }

    /// Replaces the request the page serves with `next` (ADR 0146): the
    /// earlier request's scripts are removed, counted on their way out, so
    /// that a navigation of the next request runs without them.
    pub(crate) fn replace_request(&self, next: PageId) {
        let earlier = {
            let mut frames = lock(&self.shared.frames);
            self.shared.page_id.send_replace(next);
            std::mem::take(&mut frames.live)
        };
        if earlier.is_empty() {
            return;
        }
        let leaving = self.begin();
        let scripts = self.clone();
        self.shared.run.spawn(async move {
            join_all(earlier.into_values().map(|live| scripts.removed(live))).await;
            drop(leaving);
        });
    }

    /// Adds the live scripts to the frame `cdp` reaches until it is `gone`,
    /// held before it runs.
    async fn attached(&self, cdp: CdpPage, gone: CancellationToken) {
        let sources: Vec<(u64, String)> = {
            let mut frames = lock(&self.shared.frames);
            frames.iframes.retain(|(_, gone)| !gone.is_cancelled());
            frames.iframes.push((cdp.clone(), gone));
            frames
                .live
                .iter()
                .map(|(key, live)| (*key, live.source.clone()))
                .collect()
        };
        for (key, source) in sources {
            self.hold(key, &cdp, source).await;
        }
    }

    /// Makes live `script`, whose source is `source`, now registered on the
    /// page's own session for the request `page_id`, and adds it to the
    /// page's iframes, those attached now and later; answers its key. One
    /// whose request was replaced meanwhile is removed instead: none.
    async fn add(&self, script: InitScriptId, source: String, page_id: &PageId) -> Option<u64> {
        let live = {
            let mut frames = lock(&self.shared.frames);
            (*self.shared.page_id.borrow() == *page_id).then(|| {
                let key = frames.next;
                frames.next += 1;
                frames.live.insert(
                    key,
                    Live {
                        script: script.clone(),
                        source: source.clone(),
                        held: Vec::new(),
                    },
                );
                frames.iframes.retain(|(_, gone)| !gone.is_cancelled());
                let iframes: Vec<CdpPage> =
                    frames.iframes.iter().map(|(cdp, _)| cdp.clone()).collect();
                (key, iframes)
            })
        };
        let Some((key, iframes)) = live else {
            self.shared.cdp.remove_init_script(script).await;
            return None;
        };
        for cdp in iframes {
            self.hold(key, &cdp, source.clone()).await;
        }
        Some(key)
    }

    /// Adds the script `key`, whose source is `source`, on the iframe `cdp`
    /// reaches, unless it was removed meanwhile.
    async fn hold(&self, key: u64, cdp: &CdpPage, source: String) {
        let script = match cdp.add_init_script(source, PAGE_ADD_INIT_SCRIPT.name).await {
            Ok(script) => script,
            Err(error) => {
                tracing::debug!(%error, "an out-of-process iframe went before it took an init script");
                return;
            }
        };
        let kept = match lock(&self.shared.frames).live.get_mut(&key) {
            Some(live) => {
                live.held.push((cdp.clone(), script.clone()));
                true
            }
            None => false,
        };
        if !kept {
            cdp.remove_init_script(script).await;
        }
    }

    /// Removes the script `live`, no longer live, from the page and from
    /// every iframe that holds it.
    async fn removed(&self, live: Live) {
        self.shared.cdp.remove_init_script(live.script).await;
        for (cdp, script) in live.held {
            cdp.remove_init_script(script).await;
        }
    }
}

/// One init script on its way in or out of its page.
pub(crate) struct Pending {
    scripts: InitScripts,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.scripts
            .shared
            .pending
            .send_modify(|pending| *pending = pending.saturating_sub(1));
    }
}

/// An init script registered on its page for a request, removed once
/// dropped unless its page ended or its request was replaced, which removed
/// it: the removal is counted on its way out, so a navigation sent after
/// the drop runs without the script.
pub(crate) struct Registered {
    scripts: InitScripts,
    /// Its key among its page's live scripts; none once nothing is left to
    /// remove.
    key: Option<u64>,
    /// The request it was registered for.
    page_id: PageId,
}

impl Registered {
    /// Keeps the script, whose page ended with it: nothing is left to
    /// remove.
    pub(crate) fn ended(mut self) {
        self.key = None;
    }

    /// The wait until the request it was registered for was replaced, which
    /// removed it; at once when it was before its registration.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the wait.
    pub(crate) fn request_ended(&self) -> impl Future<Output = ()> + Send + use<> {
        let mut page_id = self.scripts.shared.page_id.subscribe();
        let own = self.page_id.clone();
        async move {
            let _ = page_id.wait_for(|now| *now != own).await;
        }
    }
}

impl Drop for Registered {
    fn drop(&mut self) {
        let Some(key) = self.key.take() else {
            return;
        };
        let Some(live) = lock(&self.scripts.shared.frames).live.remove(&key) else {
            return;
        };
        let leaving = self.scripts.begin();
        let scripts = self.scripts.clone();
        self.scripts.shared.run.spawn(async move {
            scripts.removed(live).await;
            drop(leaving);
        });
    }
}
