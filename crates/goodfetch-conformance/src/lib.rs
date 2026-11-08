//! The conformance harness (ADRs 0032, 0033, 0063 and 0069): it runs a
//! suite against a target and a running Proving ground and writes one
//! `conformance` result document, and its writer turns a case log another
//! runner wrote into the same document. Every outcome comes from
//! `goodfetch-protocol`'s constructor, so a capability the suite did not
//! reach is `unproved`, never `proved`.
//!
//! It depends on no Goodfetch crate but `goodfetch-protocol`: it tests
//! through the interface, never engine internals. Its first suite, the
//! self-test, proves the Proving ground's own oracle against known-right
//! and known-wrong reference results; the protocol suite runs against any
//! target that speaks Goodfetch's JSON-RPC protocol, started from its
//! executable taken by path, and so does each generated refusal case's
//! fabricated `hello` (ADR 0066). The Cloud-readiness suite's gateway cases
//! run against a gateway reached by address (ADR 0096).

mod canaries;
mod case_list;
mod host;
mod protocol;
mod proving_ground;
mod self_test;
mod writer;

pub use case_list::{CaseList, DeclaredCapability, DeclaredCase};
pub use protocol::{
    EngineMode, EngineTarget, FabricatedHelloError, FabricatedHelloOutcome, GatewayAddress,
    GatewayAddressError, GatewaySuite, GatewaySuiteError, GatewayTarget, PROXY_PASSWORD_VARIABLE,
    ProtocolSuite, ProtocolSuiteError, StartError, SuiteError, run_gateway_suite,
    run_protocol_suite, send_fabricated_hello,
};
pub use proving_ground::{
    ProvingGround, ProvingGroundError, ProxyCredentials, Submission, Task, TaskAccount, TaskList,
};
pub use self_test::{Record, References, SelfTestError, run_self_test};
pub use writer::{CaseLogError, WriteError, read_case_log, write_document};
