//! SDK API sketch for Rust: signatures only, no implementation.
//!
//! Each ticket on the map adds its items in a module of its own and re-exports them here.
//! Function bodies are `unimplemented!()`, the sketch's one departure from `STYLE.md`.

pub mod ai;
pub mod blocking;
pub mod concurrency;
pub mod crawl;
pub mod data;
pub mod lifecycle;
pub mod models;
pub mod output;
pub mod page;
pub mod results;
pub mod sessions;
pub mod settings;
pub mod time;

pub use ai::{
    Act, AriaSnapshot, AriaSnapshotMode, Drift, EffectNotConfirmed, EffectUncertain, Expectable, Expected, Extract, ExtractMode, ExtractionIncomplete, ExtractionInvalid,
    LocatorExpectations, MissingField, ModelNotConfigured, Observe, Observed, PageExpectations, PerformedAction, PlanDrift, PlanInvalid, PlanReport, PlanStatus, PlanStepMissing,
    ResponseExpectations, StepReport, StepResult, StepSource, SuccessCheckFailed, TargetNotFound, VariablesMismatch,
    WhyNoRepair, expect,
};
pub use concurrency::{EventStream, events};
pub use crawl::{
    Crawl, CrawlCall, CrawlContext, CrawlRequest, CrawlRun, Enqueue, EnqueueLinks, Enqueued, EnqueuedLinks, FetchTier, Foxhound,
    NotAddedReason, Seed, StartCrawl, UrlPattern,
};
pub use data::{Dataset, Label, PathSegment, PayloadInvalid, PayloadSchemaChanged, Schema, SchemaIssue, schemars};
pub use lifecycle::{Collie, ConnectOptions, EngineChoice, EngineInfo, EngineKind, Goodfetch, Page, Run, Session, Start};
pub use models::{
    AppliedLimit, LimitAmount, LimitScope, LimitSetBy, ModelCall, ModelCapabilityMissing, ModelContentPart, ModelDeclaration,
    ModelFailure, ModelFormat, ModelHandlerFailed, ModelMessage, ModelOutputInvalid, ModelProvider, ModelReference, ModelRequest,
    ModelResponse, ModelTool, ModelToolCall, ModelUnavailable, ModelUsage, ModelUsageOf, ReportedTokens, RunUsage, StructuredOutput,
    StructuredOutputStrategy, UnavailableReason,
};
pub use output::{
    Compression, DatasetKeyInvalid, DatasetRecord, Export, ExportCall, ExportFormat, KeyValueStore, RecordCounts, RecordInvalid, RejectedRecord,
    SinkConfig, SinkWriteMode, StoredBytes, ValueInvalid,
};
pub use page::{
    Abort, AbortReason, Action, ApiRequest, AriaRole, BoundingBox, ByRole, CdpSession, ConsoleLocation, ConsoleMessage, Dialog,
    DialogKind, Download, ElementWait, Evaluate, FileChooser, FilePayload, Filter, Forward, FrameLocator, Fulfill, Headers, InputFiles, Keyboard,
    Locator, LocatorHandlerCall, LocatorHandlerOptions, Modifier, Mouse, MouseButton, Navigation, PageClose, PageRequest, Polling,
    Position, Query, Registration, Request, ResourceType, Response, Route, Screenshot, SelectOption, TextMatch, Touchscreen,
    WaitForFunction, WebError, WebSocket, WebSocketPayload, World, action,
};
pub use results::{
    AiWorkResult, BlockCause, Blocked, Claim, DeadlineExceeded, Ended, EndedBy, Error, ErrorCode, ErrorDetails, ErrorGroup, ErrorTarget,
    HandlerFailed, LimitSource, NotVerified, PartialOutput, Reported, CrawlRequestOutcome, CrawlRequestOutcomeKind, CrawlRequestRef, RunLayerOrigin,
    SettingValue,
    SettingsLayer, SkipReason, StopReason, TargetDescription, TargetNotUnique, Verdict,
};
pub use sessions::{
    BrowserName, ClearCookies, Cookie, CookieInput, ExportKey, HttpCredentials, HumanPauseDeclined, HumanPauseResult, Identities,
    LoggedOut, PageCheck, PageMatch, Pause, Permission, RecoveryStep, ResolvedBy, SameSite, SavedIdentity, SecretBinding,
    SecretDomainNotAllowed, SecretInputRefused, SecretPlaceholder, SecretReference, SecretRequest, SecretUnavailable, SecretUnbound,
    SessionNotSavable, SessionSource, StateFileFormat, TextInput, secret,
};
pub use settings::{ExplainedSetting, PolitenessSharing, Setting, SettingKey, Settings};
pub use time::{Call, CallFuture, Count, ElementState, Limits, LoadState, Money, UrlMatcher, Waiter};
