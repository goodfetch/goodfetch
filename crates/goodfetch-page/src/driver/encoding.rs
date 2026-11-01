//! The commands that make the guard's isolated world in a document and
//! read there the encoding the document was decoded with, never through
//! `Runtime.enable` (ADRs 0019 and 0119).

use goodfetch_cdp::generated::{page, runtime};

use super::GUARD_WORLD;

/// The expression that reads the encoding a document was decoded with.
const DOCUMENT_ENCODING: &str = "document.characterSet";

/// The command that makes, or finds by its name, the guard's isolated
/// world in `frame`'s document.
pub(crate) fn isolated_world(frame: page::FrameId) -> page::CreateIsolatedWorldParams {
    page::CreateIsolatedWorldParams {
        frame_id: frame,
        world_name: Some(GUARD_WORLD.to_owned()),
        grant_univeral_access: None,
        content_security_policy: None,
    }
}

/// The command that reads the document's encoding in the world whose
/// context is `context`, never through `Runtime.enable` (ADR 0019).
pub(crate) fn encoding_read(context: runtime::ExecutionContextId) -> runtime::EvaluateParams {
    runtime::EvaluateParams {
        expression: DOCUMENT_ENCODING.to_owned(),
        object_group: None,
        include_command_line_api: None,
        silent: Some(true),
        context_id: Some(context),
        return_by_value: Some(true),
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
    }
}

/// The encoding an [`encoding_read`] answered, when it named one.
pub(crate) fn encoding_of(evaluated: runtime::EvaluateReturns) -> Option<String> {
    evaluated
        .result
        .value
        .and_then(|value| value.as_str().map(str::to_owned))
}
