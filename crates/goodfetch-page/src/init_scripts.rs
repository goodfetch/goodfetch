//! A page's init scripts (`page.add_init_script`, ADR 0085), which its owner
//! holds (ADR 0146): each is added on the page and on its out-of-process
//! frames, and removed once its call is dropped. A navigation or a Gesture
//! sent after an init script's call waits until the script is registered,
//! or removed once its call was cancelled ([`InitScripts::settling`]).

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_engine::CallError;
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_protocol::page::functions::PAGE_ADD_INIT_SCRIPT;
use tokio::sync::{Mutex, oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::cdp::{CdpPage, InitScriptId};
use crate::driver::{Child, ChildKind, OnChild};

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

/// A live script: its source, and where each iframe holds it.
struct Live {
    source: String,
    held: Vec<(CdpPage, InitScriptId)>,
}

impl InitScripts {
    /// The init scripts of the page `cdp` reaches, none yet, added and
    /// removed by tasks of `run`.
    pub(crate) fn new(cdp: CdpPage, run: RunIdentity) -> Self {
        Self {
            shared: Arc::new(Shared {
                cdp,
                run,
                pending: watch::Sender::new(0),
                frames: Mutex::default(),
            }),
        }
    }

    /// What the init scripts do with each of the page's out-of-process
    /// frames, which `driver::follow_children` hands over: each live script
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

    /// Registers the script `source`, `pending` until it is: on a task of
    /// the Run's, so a call dropped meanwhile never leaves the script
    /// registered, as its registration, answered to no one, is removed
    /// there, counted before `pending` goes. Never answers when the Run ends
    /// under it.
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
    ) -> impl Future<Output = Result<Registered, CallError>> + Send + use<> {
        let (answer, answered) = oneshot::channel::<Result<Registered, CallError>>();
        let scripts = self.clone();
        self.shared.run.spawn(async move {
            let registered = async {
                let script = scripts
                    .shared
                    .cdp
                    .add_init_script(source.clone(), PAGE_ADD_INIT_SCRIPT.name)
                    .await?;
                // The page's out-of-process iframes take it too.
                let key = scripts.add(source).await;
                Ok::<_, CallError>(Registered {
                    scripts,
                    script: Some(script),
                    key,
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

    /// Adds the live scripts to the frame `cdp` reaches until it is `gone`,
    /// held before it runs.
    async fn attached(&self, cdp: CdpPage, gone: CancellationToken) {
        let sources: Vec<(u64, String)> = {
            let mut frames = self.shared.frames.lock().await;
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

    /// Adds `source`, a script now registered on the page's own session, to
    /// the page's iframes, those attached now and later; answers its key.
    async fn add(&self, source: String) -> u64 {
        let (key, iframes) = {
            let mut frames = self.shared.frames.lock().await;
            let key = frames.next;
            frames.next += 1;
            frames.live.insert(
                key,
                Live {
                    source: source.clone(),
                    held: Vec::new(),
                },
            );
            frames.iframes.retain(|(_, gone)| !gone.is_cancelled());
            let iframes: Vec<CdpPage> = frames.iframes.iter().map(|(cdp, _)| cdp.clone()).collect();
            (key, iframes)
        };
        for cdp in iframes {
            self.hold(key, &cdp, source.clone()).await;
        }
        key
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
        let kept = match self.shared.frames.lock().await.live.get_mut(&key) {
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

    /// Removes the script `key` from every iframe that holds it.
    async fn remove(&self, key: u64) {
        let held = self
            .shared
            .frames
            .lock()
            .await
            .live
            .remove(&key)
            .map(|live| live.held)
            .unwrap_or_default();
        for (cdp, script) in held {
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

/// An init script registered on its page, removed once dropped unless its
/// page ended: the removal is counted on its way out, so a navigation sent
/// after the drop runs without the script.
pub(crate) struct Registered {
    scripts: InitScripts,
    script: Option<InitScriptId>,
    /// Its key among its page's iframes' scripts.
    key: u64,
}

impl Registered {
    /// Keeps the script, whose page ended with it: nothing is left to
    /// remove.
    pub(crate) fn ended(mut self) {
        self.script = None;
    }
}

impl Drop for Registered {
    fn drop(&mut self) {
        let Some(script) = self.script.take() else {
            return;
        };
        let leaving = self.scripts.begin();
        let (scripts, key) = (self.scripts.clone(), self.key);
        self.scripts.shared.run.spawn(async move {
            scripts.shared.cdp.remove_init_script(script).await;
            scripts.remove(key).await;
            drop(leaving);
        });
    }
}
