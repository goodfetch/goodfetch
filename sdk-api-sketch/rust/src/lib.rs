//! SDK API sketch for Rust: signatures only, no implementation.
//!
//! Each ticket on the map adds its items in a module of its own and re-exports them here.
//! Function bodies are `unimplemented!()`, the sketch's one departure from `STYLE.md`.

pub mod lifecycle;
pub mod results;
pub mod time;

pub use lifecycle::{Collie, ConnectOptions, EngineChoice, EngineInfo, EngineKind, Goodfetch, Page, Run, Start};
pub use results::{
    AiWorkResult, BlockCause, Blocked, Claim, DeadlineExceeded, Ended, Error, ErrorCode, ErrorDetails, ErrorGroup, ErrorTarget,
    HandlerFailed, LimitSource, NotVerified, PartialOutput, Reported, RequestOutcome, RequestOutcomeKind, SettingValue,
    SettingsLayer, SkipReason, StopReason, TargetDescription, TargetNotUnique, Verdict,
};
pub use time::{
    Call, CallFuture, Count, ElementState, Limits, LoadState, Locator, Money, Request, Response, UrlMatcher, Waiter,
};
