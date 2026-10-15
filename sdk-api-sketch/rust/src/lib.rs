//! SDK API sketch for Rust: signatures only, no implementation.
//!
//! Each ticket on the map adds its items in a module of its own and re-exports them here.
//! Function bodies are `unimplemented!()`, the sketch's one departure from `STYLE.md`.

pub mod blocking;
pub mod concurrency;
pub mod data;
pub mod lifecycle;
pub mod page;
pub mod results;
pub mod sessions;
pub mod settings;
pub mod time;

pub use concurrency::{EventStream, Record, events};
pub use data::{Dataset, Label, PathSegment, PayloadInvalid, PayloadSchemaChanged, Schema, SchemaIssue, schemars};
pub use lifecycle::{Collie, ConnectOptions, EngineChoice, EngineInfo, EngineKind, Goodfetch, Page, Run, Session, Start};
pub use page::{
    Abort, AbortReason, Action, ApiRequest, AriaRole, BoundingBox, ByRole, CdpSession, ConsoleLocation, ConsoleMessage, Dialog,
    DialogKind, Download, Evaluate, FileChooser, FilePayload, Filter, Forward, FrameLocator, Fulfill, Headers, InputFiles, Keyboard,
    Locator, LocatorHandlerCall, LocatorHandlerOptions, Modifier, Mouse, MouseButton, Navigation, PageClose, PageRequest, Polling,
    Position, Query, Registration, Request, ResourceType, Response, Route, Screenshot, SelectOption, TextMatch, Touchscreen,
    WaitForFunction, WebError, WebSocket, WebSocketPayload, World, action,
};
pub use results::{
    AiWorkResult, BlockCause, Blocked, Claim, DeadlineExceeded, Ended, EndedBy, Error, ErrorCode, ErrorDetails, ErrorGroup, ErrorTarget,
    HandlerFailed, LimitSource, NotVerified, PartialOutput, Reported, RequestOutcome, RequestOutcomeKind, RunLayerOrigin,
    SettingValue,
    SettingsLayer, SkipReason, StopReason, TargetDescription, TargetNotUnique, Verdict,
};
pub use sessions::{
    BrowserName, ClearCookies, Cookie, CookieInput, ExportKey, HttpCredentials, HumanPauseDeclined, HumanPauseResult, Identities,
    LoggedOut, LoginRequest, PageCheck, PageMatch, Pause, Permission, RecoveryStep, ResolvedBy, SameSite, SavedIdentity, SecretBinding,
    SecretDomainNotAllowed, SecretInputRefused, SecretPlaceholder, SecretReference, SecretRequest, SecretUnavailable, SecretUnbound,
    SessionNotSavable, SessionSource, StateFileFormat, TextInput, secret,
};
pub use settings::{ExplainedSetting, PolitenessSharing, Setting, SettingKey, Settings};
pub use time::{Call, CallFuture, Count, ElementState, Limits, LoadState, Money, UrlMatcher, Waiter};
