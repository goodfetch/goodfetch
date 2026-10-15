//! `goodfetch-collie-client` (ADR 0095): the Collie client crate, with no engine.
//!
//! Where the main types live (ADR 0095): this crate holds [`Page`], [`Locator`], [`Run`],
//! [`Session`] and the results and errors; `goodfetch-collie` holds `Collie`; `goodfetch-foxhound`
//! holds `Foxhound`, `Crawl`, `Label`, `Dataset` and `CrawlContext`, and re-exports this crate.
//!
//! It holds the hand-written runtime, Collie's catalog types and every value a Collie connection
//! hands out: Runs, Sessions, Pages, Locators, results, errors and settings. Programs depend on
//! their Product's crate, `goodfetch-collie` or `goodfetch-foxhound`, which re-exports this one;
//! only a library meant to serve both Products depends on it, with a semver range.
//!
//! SDK API sketch: signatures only, no implementation. Function bodies are `unimplemented!()`, the
//! sketch's one departure from `STYLE.md`. In the SDK, the catalog's types come from
//! `goodfetch-protocol` (ADRs 0038, 0099); the sketch defines them here.
pub mod agent;
pub mod ai;
pub mod blocking;
pub mod catalog;
pub mod concurrency;
pub mod data;
pub mod effects;
pub mod lending;
pub mod lifecycle;
pub mod models;
pub mod output;
pub mod page;
pub mod plugins;
pub mod recording;
pub mod results;
pub mod run;
pub mod sessions;
pub mod settings;
#[cfg(feature = "test-util")]
pub mod testing;
pub mod time;

pub use agent::{
    AgentAction, AgentActionOutcome, AgentCall, AgentResult, AgentStep, AgentStepOrigin, AgentTask,
    AgentTaskProgress, AgentTool, AgentToolCall, AgentToolFailed, AgentToolInvalid,
    ConfirmationCategory, SuccessCheckThrew,
};
pub use ai::{
    Act, AriaSnapshot, AriaSnapshotMode, Drift, Extract, Observe, EffectNotConfirmed, EffectUncertain, Expectable,
    Expected, ExtractMode, ExtractionIncomplete, ExtractionInvalid, LocatorExpectations,
    MissingField, ModelNotConfigured, Observed, PageExpectations, PerformedAction, PlanDrift,
    PlanInvalid, PlanReport, PlanStatus, PlanStepMissing, ResponseExpectations, StepReport,
    StepResult, StepSource, SuccessCheckFailed, TargetNotFound, VariablesMismatch, WhyNoRepair,
    expect,
};
pub use catalog::{Connection, JsonObject, RawCall};
pub use concurrency::EventStream;
pub use data::{PathSegment, Schema, SchemaIssue};
pub use effects::{
    BodyError, DeclareEffect, Effect, EffectKeyInvalid, EffectKeyReused, EffectOutputInvalid,
    EffectResult, EffectStagingRefused, KeyAnswer, OutputProblem, StagedOutput, UncertainEffect,
    UncertainWhy,
};
pub use lending::{AttachEnded, CdpEndpoint};
pub use lifecycle::{
    ConnectOptions, EngineChoice, EngineInfo, EngineKind, Page, Run, Session, Start,
};
pub use models::{
    AllowanceAmount, AppliedLimit, AppliedModelAllowance, LimitAmount, LimitPeriod, LimitReached,
    LimitSetBy, ModelAllowanceKind, ModelAllowanceReached, ModelCall, ModelCapabilityMissing,
    ModelContentPart, ModelContextExceeded, ModelDeclaration, ModelDeclined, ModelFailure,
    ModelFormat, ModelHandlerFailed, ModelMessage, ModelOutputInvalid, ModelPriceUnknown,
    ModelProvider, ModelReference, ModelRequest, ModelRequestRejected, ModelResponse, ModelTool,
    ModelToolCall, ModelToolChoice, ModelUnavailable, ModelUsage, ModelUsageOf, PriceMissing,
    ReportedTokens, RunUsage, StructuredOutput, StructuredOutputStrategy, UnavailableReason,
    UsageQuantity,
};
pub use output::{KeyValueStore, StoredBytes, ValueInvalid};
pub use page::{
    Abort, AbortReason, Action, ApiRequest, AriaRole, BoundingBox, ByRole, CdpSession,
    ConsoleLocation, ConsoleMessage, Dialog, DialogKind, Download, ElementWait, Evaluate,
    FileChooser, FilePayload, Filter, Forward, FrameLocator, Fulfill, Headers, InputFiles,
    Keyboard, Locator, LocatorHandlerCall, LocatorHandlerOptions, Modifier, Mouse, MouseButton,
    Navigation, PageClose, PageRequest, Polling, Position, Query, Registration, Request,
    ResourceType, Response, Route, Screenshot, SelectOption, TextMatch, Touchscreen,
    WaitForFunction, WebError, WebSocket, WebSocketPayload, World,
};
pub use plugins::{ProxyAnswer, ProxyCall, ProxyRequest, SdkHandlerFailed};
pub use recording::{
    Recording, RecordingInvalid, RecordingMiss, RecordingMode, RecordingReport,
    ResponseNotRecorded,
};
pub use results::{
    AiWorkResult, BlockCause, Blocked, BoxError, CallCancelled, CancelledBy, Claim, ClosedValue,
    DeadlineExceeded, Ended, EndedBy, Error, ErrorDetails, ErrorGroup, ErrorTarget, FileWriteFailed,
    FileWriteReason, FileWriteSide, LimitSource, NotVerified, PartialOutput, PathNotWritable,
    Reported, ResponseTruncated, RunLayerOrigin, ScriptedModelDetails, SettingValue, SettingsLayer,
    StopReason, TargetDescription, TargetNotUnique, TooManyRedirects, Verdict,
};
pub use run::{
    AbortedBy, Checkpoint, Completeness, KeyValueStoreReader, Lifecycle, NothingDone,
    PauseRaisedBy, PluginRun, RunFailure, RunInputDifference, RunProgress, RunReport,
    RunStopReason, RunTrace, RunView, RunWarning, StartedAs, VerdictReason, WaitingPause,
};
pub use sessions::{
    ArtifactDownloadFailed, ArtifactUnavailableOffline, BrowserName, BrowserNotInstalled,
    CdpUrlAnswer, CdpUrlNotAllowed, CdpUrlNotBrowser, CdpUrlUnauthorized, CdpUrlUnreachable,
    ClearCookies, Cookie, CookieInput, ExportKey, HttpCredentials, HumanPauseDeclined,
    HumanPauseResult, Identities, LoggedOut, PageCheck, PageMatch, Pause, PauseUntil, Permission,
    PinnedArtifact, PinnedBy, RecoveryStep, ResolvedBy, SameSite, SavedIdentity, SecretBinding,
    SecretDomainNotAllowed, SecretInputRefused, SecretPlaceholder, SecretReference, SecretRequest,
    SecretUnavailable, SecretUnbound, SessionNotSavable, SessionSource, StateFileFormat, TextInput,
    WebStorage, secret,
};
pub use time::{Call, CallFuture, Count, ElementState, Limits, LoadState, Money, UrlMatcher, Waiter};
pub use concurrency::events;
pub use data::schemars;
pub use settings::{ExplainedSetting, PolitenessSharing, Setting, SettingKey, Settings};
// Open value types (ADR 0099), written by `open_value_type!`.
pub use models::{LimitScope, UsageKind};
pub use plugins::HookStage;
pub use results::ErrorCode;
