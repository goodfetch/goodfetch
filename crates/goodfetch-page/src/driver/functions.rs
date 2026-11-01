//! The commands a page function's evaluation sends (ADR 0085): its
//! isolated world, the engine's own apart from the guard's, so a page
//! function neither sees the page's globals nor touches the guard's state,
//! and its evaluation, in that world or in the main world of the page's
//! main frame, never through `Runtime.enable` (ADR 0019).

use goodfetch_cdp::generated::{page, runtime};

/// The isolated world page functions run in on request.
const PAGE_FUNCTION_WORLD: &str = "goodfetch-page-functions";

/// The command that makes, or finds by its name, the page functions'
/// isolated world in `frame`'s document.
pub(crate) fn page_function_world(frame: page::FrameId) -> page::CreateIsolatedWorldParams {
    page::CreateIsolatedWorldParams {
        frame_id: frame,
        world_name: Some(PAGE_FUNCTION_WORLD.to_owned()),
        grant_univeral_access: None,
        content_security_policy: None,
    }
}

/// The command that evaluates `expression` in the world whose context is
/// `context`, or in the main world of the page's main frame when none, its
/// promise awaited and its value returned by value.
pub(crate) fn evaluation(
    expression: &str,
    context: Option<runtime::ExecutionContextId>,
) -> runtime::EvaluateParams {
    runtime::EvaluateParams {
        expression: expression.to_owned(),
        object_group: None,
        include_command_line_api: None,
        silent: Some(true),
        context_id: context,
        return_by_value: Some(true),
        generate_preview: None,
        user_gesture: None,
        await_promise: Some(true),
        throw_on_side_effect: None,
        timeout: None,
        disable_breaks: None,
        repl_mode: None,
        allow_unsafe_eval_blocked_by_csp: None,
        unique_context_id: None,
        serialization_options: None,
    }
}
