//! SDK API sketch for Rust: signatures only, no implementation.
//!
//! Each ticket on the map adds its items in a module of its own and re-exports them here.
//! Function bodies are `unimplemented!()`, the sketch's one departure from `STYLE.md`.

pub mod blocking;
pub mod concurrency;
pub mod data;
pub mod lifecycle;
pub mod results;
pub mod settings;
pub mod time;

pub use concurrency::{ConsoleMessage, EventStream, Record, Route, events};
pub use data::{Dataset, Label, PathSegment, PayloadInvalid, PayloadSchemaChanged, Schema, SchemaIssue, schemars};
pub use lifecycle::{Collie, ConnectOptions, EngineChoice, EngineInfo, EngineKind, Goodfetch, Page, Run, Session, Start};
pub use results::{
    AiWorkResult, BlockCause, Blocked, Claim, DeadlineExceeded, Ended, EndedBy, Error, ErrorCode, ErrorDetails, ErrorGroup, ErrorTarget,
    HandlerFailed, LimitSource, NotVerified, PartialOutput, Reported, RequestOutcome, RequestOutcomeKind, RunLayerOrigin,
    SettingValue,
    SettingsLayer, SkipReason, StopReason, TargetDescription, TargetNotUnique, Verdict,
};
pub use settings::{ExplainedSetting, Setting, SettingKey, Settings};
pub use time::{
    Call, CallFuture, Count, ElementState, Limits, LoadState, Locator, Money, Request, Response, UrlMatcher, Waiter,
};
