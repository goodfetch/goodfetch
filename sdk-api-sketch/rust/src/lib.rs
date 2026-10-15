//! SDK API sketch for Rust: signatures only, no implementation.
//!
//! Each ticket on the map adds its items in a module of its own and re-exports them here.
//! Function bodies are `unimplemented!()`, the sketch's one departure from `STYLE.md`.

pub mod agent;
pub mod ai;
pub mod blocking;
pub mod catalog;
pub mod concurrency;
pub mod crawl;
pub mod data;
pub mod effects;
pub mod hooks;
pub mod lifecycle;
pub mod models;
pub mod output;
pub mod page;
pub mod recording;
pub mod results;
pub mod run;
pub mod sessions;
pub mod settings;
#[cfg(feature = "test-util")]
pub mod testing;
pub mod time;

pub use agent::{
    AgentAction, AgentActionOutcome, AgentCall, AgentResult, AgentStep, AgentStepOrigin, AgentTask, AgentTool, AgentToolFailed,
    AgentTaskProgress, AgentToolInvalid, ConfirmationCategory, SuccessCheckThrew, BoxError,
};
pub use ai::{
    Act, AriaSnapshot, AriaSnapshotMode, Drift, EffectNotConfirmed, EffectUncertain, Expectable, Expected, Extract, ExtractMode, ExtractionIncomplete, ExtractionInvalid,
    LocatorExpectations, MissingField, ModelNotConfigured, Observe, Observed, PageExpectations, PerformedAction, PlanDrift, PlanInvalid, PlanReport, PlanStatus, PlanStepMissing,
    ResponseExpectations, StepReport, StepResult, StepSource, SuccessCheckFailed, TargetNotFound, VariablesMismatch,
    WhyNoRepair, expect,
};
pub use catalog::{Connection, JsonObject, RawCall};
pub use concurrency::{EventStream, events};
pub use crawl::{
    Crawl, CrawlCall, CrawlContext, CrawlRequest, CrawlRun, Enqueue, EnqueueLinks, Enqueued, EnqueuedLinks, FetchTier, Foxhound, LabelDeclaration,
    LinkInfo, NotAddedReason, Seed, StartCrawl, UrlPattern,
};
pub use data::{Dataset, Label, PathSegment, PayloadInvalid, PayloadSchemaChanged, Schema, SchemaIssue, schemars};
pub use effects::{
    BodyError, DeclareEffect, Effect, EffectKeyInvalid, EffectKeyReused, EffectOutputInvalid, EffectResult, KeyAnswer, OutputProblem,
    UncertainEffect, UncertainWhy,
};
pub use hooks::{
    AfterResponseContext, BackendRequest, BackendResponse, BeforeRequestContext, FetchCall, HandlerError, HookAnswer, HookCall, HeaderForbidden, HookDuplicate, HookFailed, HookIneffective, HookOn,
    HookStage, PageStageContext, ProxyAnswer, ProxyCall, ProxyRequest, RecordCommitContext, RecordHookOn, RequestChange, ResponseChange,
    SdkHandlerFailed, SinkCall, UntypedRecord,
};
pub use lifecycle::{Collie, ConnectOptions, EngineChoice, EngineInfo, EngineKind, Goodfetch, Page, Run, Session, Start};
pub use models::{
    AppliedLimit, LimitAmount, LimitScope, LimitSetBy, ModelCall, ModelCapabilityMissing, ModelContentPart, ModelContextExceeded, ModelDeclined, ModelDeclaration,
    ModelFailure, ModelFormat, ModelHandlerFailed, ModelMessage, ModelOutputInvalid, ModelPriceUnknown, ModelProvider, ModelReference, ModelRequest, ModelRequestRejected,
    ModelResponse, ModelTool, ModelToolCall, ModelToolChoice, ModelUnavailable, ModelUsage, ModelUsageOf, ReportedTokens, RunUsage, StructuredOutput,
    StructuredOutputStrategy, PriceMissing, UnavailableReason,
};
pub use output::{
    Compression, DatasetDuplicate, DatasetKeyInvalid, DatasetRecord, DatasetUnknown, Export, ExportCall, ExportFormat, KeyValueStore, RecordCounts, RecordInvalid, RejectedRecord,
    SinkConfig, SinkWriteMode, StoredBytes, ValueInvalid,
};
pub use page::{
    Abort, AbortReason, Action, ApiRequest, AriaRole, BoundingBox, ByRole, CdpSession, ConsoleLocation, ConsoleMessage, Dialog,
    DialogKind, Download, ElementWait, Evaluate, FileChooser, FilePayload, Filter, Forward, FrameLocator, Fulfill, Headers, InputFiles, Keyboard,
    Locator, LocatorHandlerCall, LocatorHandlerOptions, Modifier, Mouse, MouseButton, Navigation, PageClose, PageRequest, Polling,
    Position, Query, Registration, Request, ResourceType, Response, Route, Screenshot, SelectOption, TextMatch, Touchscreen,
    WaitForFunction, WebError, WebSocket, WebSocketPayload, World, action,
};
pub use recording::{Recording, RecordingInvalid, RecordingMiss, RecordingMode, RecordingReport, ResponseNotRecorded};
pub use results::{
    AiWorkResult, BlockCause, Blocked, Claim, DeadlineExceeded, Ended, EndedBy, Error, ErrorCode, ErrorDetails, ErrorGroup, ErrorTarget,
    HandlerFailed, LimitSource, NotVerified, PartialOutput, Reported, CrawlRequestOutcome, CrawlRequestOutcomeKind, CrawlRequestRef, RunLayerOrigin,
    SettingValue,
    ScriptedModelDetails, SettingsLayer, SkipReason, StopReason, TargetDescription, TargetNotUnique, Verdict,
};
pub use run::{
    AbortedBy, Checkpoint, Completeness, CrawlConcurrency, CrawlRequestCounts, CrawlRequestTotals, CrawlRunProgress, CrawlRunReport,
    CrawlRunView, DatasetTotals, EndedRequestBreakdown, GovernorState, InFlightRequest, KeyValueStoreReader, LearnedRule, Lifecycle,
    NothingDone, OutcomeKind, OutcomesFilter, PauseRaisedBy, PluginRun, RequestPhase, RetryFailed, RunFailure, RunInputDifference, RunProgress,
    RunReport, RunStopReason, RunView, RunWarning, SinkDeliveryStatus, SitemapTotals, StartedAs, RunTrace, UnreadSitemap, VerdictReason, WaitGroup, WaitReason, WaitingDomain, WaitingPause,
};
pub use sessions::{
    BrowserName, ClearCookies, Cookie, CookieInput, ExportKey, HttpCredentials, HumanPauseDeclined, HumanPauseResult, Identities,
    LoggedOut, PageCheck, PageMatch, Pause, Permission, RecoveryStep, ResolvedBy, SameSite, SavedIdentity, SecretBinding,
    SecretDomainNotAllowed, SecretInputRefused, SecretPlaceholder, SecretReference, SecretRequest, SecretUnavailable, SecretUnbound,
    SessionNotSavable, SessionSource, StateFileFormat, TextInput, WebStorage, secret,
};
pub use settings::{ExplainedSetting, PolitenessSharing, Setting, SettingKey, Settings};
pub use time::{Call, CallFuture, Count, ElementState, Limits, LoadState, Money, UrlMatcher, Waiter};
