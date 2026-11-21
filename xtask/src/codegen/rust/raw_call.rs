//! A handle's raw call in the Rust handles (ADR 0099): `call(operation)`
//! names an operation of the handle's namespace the SDK may not generate,
//! fills the parameter its ID names, such as `page_id`, and answers the
//! crate's `RawCall`, or its blocking twin, whose params and result are the
//! wire's JSON in snake case.

use super::Twin;

/// The `call` method of a handle of `namespace` whose ID fills `param`.
pub(super) fn member(twin: Twin, namespace: &str, param: &str) -> String {
    let raw = match twin {
        Twin::Async => format!(
            "crate::RawCall {{\n        self.host.raw(operation, \"{namespace}\", \"{param}\")"
        ),
        Twin::Blocking => "crate::blocking::RawCall {\n        crate::blocking::RawCall::new(self.inner.call(operation), self.inner.host.connection())".to_owned(),
    };
    format!(
        "\n    /// The raw call of `operation`, an operation of `{namespace}` the SDK may not\n    /// generate (ADR 0099), its params and its result the wire's JSON in\n    /// snake case: its ID fills `{param}`. An operation of another namespace\n    /// fails with `invalid_params` before it is sent, as a call on a\n    /// closed value does with `closed`. It is never retried.\n    pub fn call(&self, operation: &str) -> {raw}\n    }}\n"
    )
}
