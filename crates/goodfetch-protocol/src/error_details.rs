//! Which Error codes each of the protocol's details types holds the
//! details of (ADR 0099), so an SDK's error reads them by type with
//! `details::<D>()`. A type of one code is named for it, `<Code>Details`,
//! which the tests check in the table below; the few that several codes share, or that
//! another message also carries, list their codes by hand. Its tests also
//! check that each catalog code has its `ErrorCode` constant, which needs
//! every Product's modules.

use crate::error_code::{ErrorCode, ErrorDetails};
use crate::foxhound::error_code as foxhound;

/// The Error codes whose details name `retry_after` as a point in time, the
/// time the site's `Retry-After` asks for (ADRs 0092 and 0118), which each
/// SDK's class of the code reads as its own `retryAfter` field.
pub const RETRY_AFTER_POINT_IN_TIME: &[ErrorCode] =
    &[ErrorCode::BLOCKED, foxhound::RESPONSE_UNAVAILABLE];

/// Implements [`ErrorDetails`] for each type, naming its code.
macro_rules! details {
    ($($details:path => $code:expr),+ $(,)?) => {
        $(impl ErrorDetails for $details {
            const CODES: &'static [ErrorCode] = &[$code];
        })+
    };
}

impl ErrorDetails for crate::run::RunDetails {
    const CODES: &'static [ErrorCode] = &[
        ErrorCode::RUN_NOT_FOUND,
        ErrorCode::RUN_IN_USE,
        ErrorCode::RUN_BUSY,
        ErrorCode::RUN_NOT_OWNED,
        ErrorCode::RUN_LIVE,
        ErrorCode::RUN_STATE_UNAVAILABLE,
        ErrorCode::RUN_ENDED,
    ];
}

/// `blocked`'s details as Foxhound reads them, its cause Foxhound's.
impl ErrorDetails for crate::foxhound::block::BlockedDetails {
    const CODES: &'static [ErrorCode] = &[ErrorCode::BLOCKED];
}

impl ErrorDetails for crate::run::ReachedLimit {
    const CODES: &'static [ErrorCode] = &[ErrorCode::LIMIT_REACHED];
}

details! {
    crate::browser_cache::ArtifactDownloadFailedDetails => ErrorCode::ARTIFACT_DOWNLOAD_FAILED,
    crate::browser_cache::ArtifactUnavailableOfflineDetails => ErrorCode::ARTIFACT_UNAVAILABLE_OFFLINE,
    crate::browser_cache::BrowserNotInstalledDetails => ErrorCode::BROWSER_NOT_INSTALLED,
    crate::checkpoint::FileWriteFailedDetails => ErrorCode::FILE_WRITE_FAILED,
    crate::checkpoint::FormatVersionUnsupportedDetails => ErrorCode::FORMAT_VERSION_UNSUPPORTED,
    crate::checkpoint::PathNotWritableDetails => ErrorCode::PATH_NOT_WRITABLE,
    crate::checkpoint::SessionBoundToMachineDetails => ErrorCode::SESSION_BOUND_TO_MACHINE,
    crate::connection::CallCancelledDetails => ErrorCode::CALL_CANCELLED,
    crate::connection::ConnectionOverloadedDetails => ErrorCode::CONNECTION_OVERLOADED,
    crate::connection::DeadlineExceededDetails => ErrorCode::DEADLINE_EXCEEDED,
    crate::connection::EdgeHandlersUnsupportedDetails => ErrorCode::EDGE_HANDLERS_UNSUPPORTED,
    crate::connection::InternalDetails => ErrorCode::INTERNAL,
    crate::connection::NotInProductDetails => ErrorCode::NOT_IN_PRODUCT,
    crate::connection::ProtocolVersionUnsupportedDetails => ErrorCode::PROTOCOL_VERSION_UNSUPPORTED,
    crate::connection::TooManyCallsDetails => ErrorCode::TOO_MANY_CALLS,
    crate::connection::EnginePanickedDetails => ErrorCode::ENGINE_PANICKED,
    crate::connection::EngineExitedDetails => ErrorCode::ENGINE_EXITED,
    crate::daemon::DaemonDrainingDetails => ErrorCode::DAEMON_DRAINING,
    crate::daemon::DaemonUnreachableDetails => ErrorCode::DAEMON_UNREACHABLE,
    crate::daemon::NotADaemonDetails => ErrorCode::NOT_A_DAEMON,
    crate::dns::AddressFamilyUnavailableDetails => ErrorCode::ADDRESS_FAMILY_UNAVAILABLE,
    crate::dns::NameLookupFailedDetails => ErrorCode::NAME_LOOKUP_FAILED,
    crate::dns::NameNotResolvedDetails => ErrorCode::NAME_NOT_RESOLVED,
    crate::egress::CdpUrlNotAllowedDetails => ErrorCode::CDP_URL_NOT_ALLOWED,
    crate::egress::EgressRefusedDetails => ErrorCode::EGRESS_REFUSED,
    crate::egress::LocalAddressUnavailableDetails => ErrorCode::LOCAL_ADDRESS_UNAVAILABLE,
    crate::events::ListenerEventsLostDetails => ErrorCode::LISTENER_EVENTS_LOST,
    crate::executables::ExecutableBlockedBySmartAppControlDetails => ErrorCode::EXECUTABLE_BLOCKED_BY_SMART_APP_CONTROL,
    crate::foxhound::crawl::LabelUnknownDetails => foxhound::LABEL_UNKNOWN,
    crate::foxhound::crawl::ResponseUnavailableDetails => foxhound::RESPONSE_UNAVAILABLE,
    crate::foxhound::crawl::StagingFullDetails => foxhound::STAGING_FULL,
    crate::foxhound::dataset::DatasetDuplicateDetails => foxhound::DATASET_DUPLICATE,
    crate::foxhound::dataset::DatasetKeyInvalidDetails => foxhound::DATASET_KEY_INVALID,
    crate::foxhound::dataset::DatasetUnknownDetails => foxhound::DATASET_UNKNOWN,
    crate::foxhound::dataset::RecordInvalidDetails => foxhound::RECORD_INVALID,
    crate::foxhound::fetch::BrowserRequiredDetails => foxhound::BROWSER_REQUIRED,
    crate::foxhound::fetch::ResponseNotDocumentDetails => foxhound::RESPONSE_NOT_DOCUMENT,
    crate::foxhound::frontier::PayloadInvalidDetails => foxhound::PAYLOAD_INVALID,
    crate::foxhound::frontier::PayloadSchemaChangedDetails => foxhound::PAYLOAD_SCHEMA_CHANGED,
    crate::foxhound::hook::HookDuplicateDetails => foxhound::HOOK_DUPLICATE,
    crate::foxhound::hook::HookIneffectiveDetails => foxhound::HOOK_INEFFECTIVE,
    crate::foxhound::hook::HookUnknownDetails => foxhound::HOOK_UNKNOWN,
    crate::foxhound::sql_tab::QueryInvalidDetails => foxhound::QUERY_INVALID,
    crate::foxhound::sql_tab::QueryMemoryExceededDetails => foxhound::QUERY_MEMORY_EXCEEDED,
    crate::governor::PageMemoryExceededDetails => ErrorCode::PAGE_MEMORY_EXCEEDED,
    crate::hook::HookFailedDetails => ErrorCode::HOOK_FAILED,
    crate::hostile_input::HeaderValueInvalidDetails => ErrorCode::HEADER_VALUE_INVALID,
    crate::hostile_input::PathUnsafeDetails => ErrorCode::PATH_UNSAFE,
    crate::key_value_store::ValueInvalidDetails => ErrorCode::VALUE_INVALID,
    crate::page::BlockedDetails => ErrorCode::BLOCKED,
    crate::page::BodyNotFoundDetails => ErrorCode::BODY_NOT_FOUND,
    crate::page::BrowserLaunchFailedDetails => ErrorCode::BROWSER_LAUNCH_FAILED,
    crate::page::LoggedOutDetails => ErrorCode::LOGGED_OUT,
    crate::page::NavigationFailedDetails => ErrorCode::NAVIGATION_FAILED,
    crate::page::NavigationInterruptedDetails => ErrorCode::NAVIGATION_INTERRUPTED,
    crate::page::PageCheckFailedDetails => ErrorCode::PAGE_CHECK_FAILED,
    crate::page::PageCrashedDetails => ErrorCode::PAGE_CRASHED,
    crate::page::PatternUnsupportedDetails => ErrorCode::PATTERN_UNSUPPORTED,
    crate::page::ResponseTruncatedDetails => ErrorCode::RESPONSE_TRUNCATED,
    crate::page::SitePreferencesRefusedDetails => ErrorCode::SITE_PREFERENCES_REFUSED,
    crate::page::TargetNotUniqueDetails => ErrorCode::TARGET_NOT_UNIQUE,
    crate::page::ai_work::NotVerifiedDetails => ErrorCode::NOT_VERIFIED,
    crate::page::TooManyRedirectsDetails => ErrorCode::TOO_MANY_REDIRECTS,
    crate::page::cdp_url::CdpUrlNotBrowserDetails => ErrorCode::CDP_URL_NOT_BROWSER,
    crate::page::cdp_url::CdpUrlUnauthorizedDetails => ErrorCode::CDP_URL_UNAUTHORIZED,
    crate::page::cdp_url::CdpUrlUnreachableDetails => ErrorCode::CDP_URL_UNREACHABLE,
    crate::page::dialog::DialogAlreadyHandledDetails => ErrorCode::DIALOG_ALREADY_HANDLED,
    crate::page::events::BodyUnavailableDetails => ErrorCode::BODY_UNAVAILABLE,
    crate::page::functions::PageFunctionFailedDetails => ErrorCode::PAGE_FUNCTION_FAILED,
    crate::page::functions::PageValueInvalidDetails => ErrorCode::PAGE_VALUE_INVALID,
    crate::page::proxy::ProxyAuthFailedDetails => ErrorCode::PROXY_AUTH_FAILED,
    crate::page::proxy::ProxyProviderUnavailableDetails => ErrorCode::PROXY_PROVIDER_UNAVAILABLE,
    crate::page::proxy::ProxyRelayUnavailableDetails => ErrorCode::PROXY_RELAY_UNAVAILABLE,
    crate::page::proxy::ProxyUnavailableDetails => ErrorCode::PROXY_UNAVAILABLE,
    crate::page::route::PageHandlerFailedDetails => ErrorCode::PAGE_HANDLER_FAILED,
    crate::page::route::RouteAlreadyHandledDetails => ErrorCode::ROUTE_ALREADY_HANDLED,
    crate::response_limits::ResponseLimitExceededDetails => ErrorCode::RESPONSE_LIMIT_EXCEEDED,
    crate::run::ResourcesExhaustedDetails => ErrorCode::RESOURCES_EXHAUSTED,
    crate::run::RunConflictDetails => ErrorCode::RUN_CONFLICT,
    crate::run::RunIdInvalidDetails => ErrorCode::RUN_ID_INVALID,
    crate::run::RunInterruptedDetails => ErrorCode::RUN_INTERRUPTED,
    crate::secrets::RunKeyMismatchDetails => ErrorCode::RUN_KEY_MISMATCH,
    crate::secrets::SecretDomainNotAllowedDetails => ErrorCode::SECRET_DOMAIN_NOT_ALLOWED,
    crate::secrets::SecretInputRefusedDetails => ErrorCode::SECRET_INPUT_REFUSED,
    crate::secrets::SecretUnavailableDetails => ErrorCode::SECRET_UNAVAILABLE,
    crate::secrets::SecretUnboundDetails => ErrorCode::SECRET_UNBOUND,
    crate::security::BrowserSandboxUnavailableDetails => ErrorCode::BROWSER_SANDBOX_UNAVAILABLE,
    crate::settings::CapabilityUnavailableDetails => ErrorCode::CAPABILITY_UNAVAILABLE,
    crate::settings::SettingInvalidDetails => ErrorCode::SETTING_INVALID,
    crate::settings::SettingLockedDetails => ErrorCode::SETTING_LOCKED,
    crate::settings::SettingNotApplicableDetails => ErrorCode::SETTING_NOT_APPLICABLE,
    crate::settings::SettingUnknownDetails => ErrorCode::SETTING_UNKNOWN,
    crate::settings::edit::PolicyConflictDetails => ErrorCode::POLICY_CONFLICT,
    crate::tls::CertificateChangedDetails => ErrorCode::CERTIFICATE_CHANGED,
    crate::tls::CertificateInvalidDetails => ErrorCode::CERTIFICATE_INVALID,
    crate::usage::LimitNotUnderstoodDetails => ErrorCode::LIMIT_NOT_UNDERSTOOD,
    crate::usage::LimitRaiseRefusedDetails => ErrorCode::LIMIT_RAISE_REFUSED,
}

#[cfg(test)]
mod tests;
