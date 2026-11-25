//! The primitives that have a page run a script before each document's own,
//! and run it no more.

use goodfetch_cdp::generated::page;
use goodfetch_engine::CallError;

use super::{CdpPage, TargetSession};
use crate::errors::Call;

/// The browser's ID of an init script, on the session it was added on.
#[derive(Clone, Debug)]
pub(crate) struct InitScriptId(page::ScriptIdentifier);

impl CdpPage {
    /// The page's out-of-process frame on `session`, as a page whose
    /// primitives reach that frame.
    pub(crate) fn frame(&self, session: &TargetSession) -> Self {
        Self {
            page: self.page.on_child(session.clone()),
        }
    }

    /// Adds `source` to the scripts run in the main world of every document
    /// the page loads from now on, the frames of its process included,
    /// before their own scripts; a failure raises the code of the operation
    /// `operation`.
    ///
    /// # Errors
    ///
    /// The code of `operation` when the page cannot take it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the script added.
    pub(crate) async fn add_init_script(
        &self,
        source: String,
        operation: &'static str,
    ) -> Result<InitScriptId, CallError> {
        let added = self
            .page
            .command_for(
                &page::AddScriptToEvaluateOnNewDocumentParams {
                    source,
                    world_name: None,
                    include_command_line_api: None,
                    run_immediately: None,
                },
                &Call::Action { operation },
            )
            .await?;
        Ok(InitScriptId(added.identifier))
    }

    /// Removes the init script `script`; a page gone has none left.
    pub(crate) async fn remove_init_script(&self, script: InitScriptId) {
        let removed = self
            .taken(&page::RemoveScriptToEvaluateOnNewDocumentParams {
                identifier: script.0,
            })
            .await;
        if let Err(failure) = removed {
            tracing::debug!(%failure, "a page went before its init script was removed");
        }
    }
}
