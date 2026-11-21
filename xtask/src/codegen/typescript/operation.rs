//! The async method of an operations class in the TypeScript layer (ADR
//! 0099): its `TSDoc`, its markers' warnings, and the deprecated-key check
//! of a run layer it takes as raw JSON.

use std::fmt::Write as _;

use super::super::layer::{Layer, Operation};
use super::super::naming::camel;
use super::{doc, settings, warnings};

/// The async method of the operations class sending `operation`, with its
/// `TSDoc` and its warnings: its markers', and the deprecated keys of a run
/// layer it takes as raw JSON, as a typed member's settings are (ADR 0099).
pub(super) fn operation_method(layer: &Layer, operation: &Operation) -> String {
    let mut text = doc(
        "  ",
        &operation.description,
        operation.stability,
        operation.deprecated.as_ref(),
    );
    let method = camel(&operation.name);
    let (params, result) = (&operation.params.0, &operation.result.0);
    let mut warn = warnings(operation);
    if layer.takes_raw_settings(operation) {
        let _ = writeln!(
            warn,
            "    {}(params.settings);",
            settings::WARN_DEPRECATED_SETTINGS
        );
    }
    if operation.cancellable {
        let _ = write!(
            text,
            "  {method}(params: {params}, options: CallOptions = {{}}): Promise<{result}> {{\n{warn}    return this.#caller.call({:?}, params, options);\n  }}\n",
            operation.name
        );
    } else {
        let _ = write!(
            text,
            "  {method}(params: {params}): Promise<{result}> {{\n{warn}    return this.#caller.call({:?}, params, {{}});\n  }}\n",
            operation.name
        );
    }
    text
}
