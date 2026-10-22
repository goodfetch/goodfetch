//! A call into an isolated world as CDP takes it: `Runtime.callFunctionOn`
//! on the world's execution context, with one argument, its answer awaited
//! and returned by value, never through `Runtime.enable` (ADR 0019). Pure,
//! so the check-and-arm round trip's benchmark sends the very command the
//! guard sends (ADR 0071).

use goodfetch_cdp::generated::runtime::{CallArgument, CallFunctionOnParams, ExecutionContextId};
use serde_json::Value;

/// The command that calls `function`, a JavaScript function's source,
/// with `argument` in the world whose context is `context`.
pub(crate) fn call(
    function: &str,
    argument: Value,
    context: ExecutionContextId,
) -> CallFunctionOnParams {
    CallFunctionOnParams {
        function_declaration: function.to_owned(),
        object_id: None,
        arguments: Some(vec![CallArgument {
            value: Some(argument),
            unserializable_value: None,
            object_id: None,
        }]),
        silent: Some(true),
        return_by_value: Some(true),
        generate_preview: None,
        user_gesture: None,
        await_promise: Some(true),
        execution_context_id: Some(context),
        object_group: None,
        throw_on_side_effect: None,
        unique_context_id: None,
        serialization_options: None,
    }
}
