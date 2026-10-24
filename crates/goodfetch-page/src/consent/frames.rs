//! The frames of one page as the Consent prompt runner knows them: each
//! target's session (the page's, and each out-of-process iframe's, attached
//! under its parent) and each frame's guard world in its current document.
//! Pure, so the way up from a frame to the page, which a click's point is
//! read along, is tested without a browser.

use std::collections::BTreeMap;

use goodfetch_cdp::SessionId as TargetSession;
use goodfetch_cdp::generated::network::LoaderId;
use goodfetch_cdp::generated::page::FrameId;
use goodfetch_cdp::generated::runtime::ExecutionContextId;

/// One frame's document, by the guard's world in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Frame {
    /// The session of its target.
    pub(super) session: TargetSession,
    /// The frame's ID.
    pub(super) id: FrameId,
    /// The guard's world in its current document.
    pub(super) context: ExecutionContextId,
}

/// The page's frames.
pub(super) struct Frames {
    /// Each target's main frame: the page's own, and an iframe target's,
    /// whose ID is its target's.
    mains: BTreeMap<TargetSession, FrameId>,
    /// Each iframe target's parent.
    parents: BTreeMap<TargetSession, TargetSession>,
    /// Each frame's guard world in its current document.
    contexts: BTreeMap<(TargetSession, FrameId), ExecutionContextId>,
    /// The browser's ID of each frame's current document's load.
    loaders: BTreeMap<(TargetSession, FrameId), LoaderId>,
}

impl Frames {
    /// The frames of the page whose session is `page` and main frame
    /// `main`.
    pub(super) fn new(page: TargetSession, main: FrameId) -> Self {
        Self {
            mains: BTreeMap::from([(page, main)]),
            parents: BTreeMap::new(),
            contexts: BTreeMap::new(),
            loaders: BTreeMap::new(),
        }
    }

    /// The iframe target `child`, whose ID is `target`, attached under
    /// `parent`.
    pub(super) fn attach(&mut self, parent: &TargetSession, child: &TargetSession, target: String) {
        self.mains.insert(child.clone(), target);
        self.parents.insert(child.clone(), parent.clone());
    }

    /// `frame` of `session` holds a new document, the load `loader`, whose
    /// guard world is `context`.
    pub(super) fn document(
        &mut self,
        session: &TargetSession,
        frame: FrameId,
        context: ExecutionContextId,
        loader: LoaderId,
    ) {
        self.contexts
            .insert((session.clone(), frame.clone()), context);
        self.loaders.insert((session.clone(), frame), loader);
    }

    /// The browser's ID of the load of `frame`'s current document.
    pub(super) fn loader(&self, session: &TargetSession, frame: &FrameId) -> Option<LoaderId> {
        self.loaders.get(&(session.clone(), frame.clone())).cloned()
    }

    /// The guard's world in `frame`'s current document.
    pub(super) fn context(
        &self,
        session: &TargetSession,
        frame: &FrameId,
    ) -> Option<ExecutionContextId> {
        self.contexts
            .get(&(session.clone(), frame.clone()))
            .copied()
    }

    /// The frame whose current document's guard world is `context`.
    pub(super) fn frame_of(
        &self,
        session: &TargetSession,
        context: ExecutionContextId,
    ) -> Option<Frame> {
        self.contexts
            .iter()
            .find(|((owner, _), world)| owner == session && **world == context)
            .map(|((session, frame), context)| Frame {
                session: session.clone(),
                id: frame.clone(),
                context: *context,
            })
    }

    /// The iframe target `session` detached.
    pub(super) fn forget_session(&mut self, session: &TargetSession) {
        self.mains.remove(session);
        self.parents.remove(session);
        self.contexts.retain(|(owner, _), _| owner != session);
        self.loaders.retain(|(owner, _), _| owner != session);
    }

    /// `frame` of `session` detached.
    pub(super) fn forget_frame(&mut self, session: &TargetSession, frame: &FrameId) {
        self.contexts.remove(&(session.clone(), frame.clone()));
        self.loaders.remove(&(session.clone(), frame.clone()));
    }

    /// The owners between `frame` and the page, innermost first: each
    /// frame whose owner element the click's point goes through, with the
    /// session that holds that owner. An iframe in its own target's process
    /// is read in that target's main frame's viewport, so the way up jumps
    /// to that main frame; an out-of-process iframe's owner is in its
    /// parent target. `None` for a frame of a target no longer attached.
    pub(super) fn owner_chain(&self, frame: &Frame) -> Option<Vec<(TargetSession, FrameId)>> {
        let mut chain = Vec::new();
        let mut session = frame.session.clone();
        let mut current = frame.id.clone();
        loop {
            let main = self.mains.get(&session)?;
            if current != *main {
                chain.push((
                    session.clone(),
                    std::mem::replace(&mut current, main.clone()),
                ));
                continue;
            }
            let Some(parent) = self.parents.get(&session) else {
                return Some(chain);
            };
            let above = self.mains.get(parent)?.clone();
            chain.push((parent.clone(), std::mem::replace(&mut current, above)));
            session = parent.clone();
        }
    }
}

#[cfg(test)]
mod tests;
