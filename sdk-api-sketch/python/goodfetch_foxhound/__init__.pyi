"""`goodfetch_foxhound` (ADRs 0090, 0095): Foxhound's Python SDK and its `goodfetch-foxhound` executable.

It re-exports the Collie client package, so a handler's Page is the Page a Collie program gets, and adds Foxhound's
connection, the generated layer of Foxhound's own entries and the objects only Foxhound hands out: crawls and their
contexts, Datasets, Records, Exports, Hooks and Politeness. Where a name both packages define means more here
(`Settings`, `TimeoutSettings`, `RunTimeoutSettings`, `SettingKey`, `PolitenessSettings`, `SitePreferencesSettings`,
`KnownErrorCode`, `ErrorCode`), Foxhound's own, which adds its keys or values to Collie's, is the one this package gives.
"""

from goodfetch_collie_client import AbortReason as AbortReason
from goodfetch_collie_client import ActionDone as ActionDone
from goodfetch_collie_client import ActionFailed as ActionFailed
from goodfetch_collie_client import ActionJournaled as ActionJournaled
from goodfetch_collie_client import ActionUnknown as ActionUnknown
from goodfetch_collie_client import agent_tool as agent_tool
from goodfetch_collie_client import AgentAction as AgentAction
from goodfetch_collie_client import AgentActionOutcome as AgentActionOutcome
from goodfetch_collie_client import AgentAPI as AgentAPI
from goodfetch_collie_client import AgentResult as AgentResult
from goodfetch_collie_client import AgentSettings as AgentSettings
from goodfetch_collie_client import AgentStep as AgentStep
from goodfetch_collie_client import AgentStepOrigin as AgentStepOrigin
from goodfetch_collie_client import AgentTask as AgentTask
from goodfetch_collie_client import AgentTaskProgress as AgentTaskProgress
from goodfetch_collie_client import AgentTool as AgentTool
from goodfetch_collie_client import AgentToolCall as AgentToolCall
from goodfetch_collie_client import AgentToolFailedDetails as AgentToolFailedDetails
from goodfetch_collie_client import AgentToolFailedError as AgentToolFailedError
from goodfetch_collie_client import AgentToolInvalidDetails as AgentToolInvalidDetails
from goodfetch_collie_client import AgentToolInvalidError as AgentToolInvalidError
from goodfetch_collie_client import AISettings as AISettings
from goodfetch_collie_client import AIVerbs as AIVerbs
from goodfetch_collie_client import AIWorkResult as AIWorkResult
from goodfetch_collie_client import AppliedLimit as AppliedLimit
from goodfetch_collie_client import AppliedModelAllowance as AppliedModelAllowance
from goodfetch_collie_client import AriaRole as AriaRole
from goodfetch_collie_client import AriaSnapshotMode as AriaSnapshotMode
from goodfetch_collie_client import AsyncAgentAPI as AsyncAgentAPI
from goodfetch_collie_client import AsyncAgentResult as AsyncAgentResult
from goodfetch_collie_client import AsyncAgentTask as AsyncAgentTask
from goodfetch_collie_client import AsyncAgentTool as AsyncAgentTool
from goodfetch_collie_client import AsyncAIVerbs as AsyncAIVerbs
from goodfetch_collie_client import AsyncCDPEndpoint as AsyncCDPEndpoint
from goodfetch_collie_client import AsyncCDPSession as AsyncCDPSession
from goodfetch_collie_client import AsyncCheckpoint as AsyncCheckpoint
from goodfetch_collie_client import AsyncConnection as AsyncConnection
from goodfetch_collie_client import AsyncDeclaresEffects as AsyncDeclaresEffects
from goodfetch_collie_client import AsyncDialog as AsyncDialog
from goodfetch_collie_client import AsyncDialogHandler as AsyncDialogHandler
from goodfetch_collie_client import AsyncDownload as AsyncDownload
from goodfetch_collie_client import AsyncElementReferences as AsyncElementReferences
from goodfetch_collie_client import AsyncEventInfo as AsyncEventInfo
from goodfetch_collie_client import AsyncEventIterator as AsyncEventIterator
from goodfetch_collie_client import AsyncFileChooser as AsyncFileChooser
from goodfetch_collie_client import AsyncFrameLocator as AsyncFrameLocator
from goodfetch_collie_client import AsyncIdentities as AsyncIdentities
from goodfetch_collie_client import AsyncKeyboard as AsyncKeyboard
from goodfetch_collie_client import AsyncKeyValueStore as AsyncKeyValueStore
from goodfetch_collie_client import AsyncKeyValueStoreReader as AsyncKeyValueStoreReader
from goodfetch_collie_client import AsyncLendsPage as AsyncLendsPage
from goodfetch_collie_client import AsyncListener as AsyncListener
from goodfetch_collie_client import AsyncLocator as AsyncLocator
from goodfetch_collie_client import AsyncLocatorExpectations as AsyncLocatorExpectations
from goodfetch_collie_client import AsyncModelProviderHandler as AsyncModelProviderHandler
from goodfetch_collie_client import AsyncMouse as AsyncMouse
from goodfetch_collie_client import AsyncPage as AsyncPage
from goodfetch_collie_client import AsyncPageAPI as AsyncPageAPI
from goodfetch_collie_client import AsyncPageEvents as AsyncPageEvents
from goodfetch_collie_client import AsyncPageExpectations as AsyncPageExpectations
from goodfetch_collie_client import AsyncPageRequest as AsyncPageRequest
from goodfetch_collie_client import AsyncPageStorage as AsyncPageStorage
from goodfetch_collie_client import AsyncPageWaits as AsyncPageWaits
from goodfetch_collie_client import AsyncPausesForHuman as AsyncPausesForHuman
from goodfetch_collie_client import AsyncPlaywrightChromium as AsyncPlaywrightChromium
from goodfetch_collie_client import AsyncProxyProviderHandler as AsyncProxyProviderHandler
from goodfetch_collie_client import AsyncRawCall as AsyncRawCall
from goodfetch_collie_client import AsyncRequest as AsyncRequest
from goodfetch_collie_client import AsyncResolveSecret as AsyncResolveSecret
from goodfetch_collie_client import AsyncResponse as AsyncResponse
from goodfetch_collie_client import AsyncResponseExpectations as AsyncResponseExpectations
from goodfetch_collie_client import AsyncRoute as AsyncRoute
from goodfetch_collie_client import AsyncRouteHandler as AsyncRouteHandler
from goodfetch_collie_client import AsyncRun as AsyncRun
from goodfetch_collie_client import AsyncRunControl as AsyncRunControl
from goodfetch_collie_client import AsyncRunEvents as AsyncRunEvents
from goodfetch_collie_client import AsyncRunTrace as AsyncRunTrace
from goodfetch_collie_client import AsyncRunView as AsyncRunView
from goodfetch_collie_client import AsyncSession as AsyncSession
from goodfetch_collie_client import AsyncSessionStateAPI as AsyncSessionStateAPI
from goodfetch_collie_client import AsyncTouchscreen as AsyncTouchscreen
from goodfetch_collie_client import AsyncWebSocket as AsyncWebSocket
from goodfetch_collie_client import AsyncWebStorage as AsyncWebStorage
from goodfetch_collie_client import AttachEndedDetails as AttachEndedDetails
from goodfetch_collie_client import AttachEndedError as AttachEndedError
from goodfetch_collie_client import BlockCause as BlockCause
from goodfetch_collie_client import BlockedDetails as BlockedDetails
from goodfetch_collie_client import BlockedError as BlockedError
from goodfetch_collie_client import BodyUnavailableError as BodyUnavailableError
from goodfetch_collie_client import BorrowableAsyncBrowser as BorrowableAsyncBrowser
from goodfetch_collie_client import BorrowableAsyncPage as BorrowableAsyncPage
from goodfetch_collie_client import BorrowableBrowser as BorrowableBrowser
from goodfetch_collie_client import BorrowablePage as BorrowablePage
from goodfetch_collie_client import BoundingBox as BoundingBox
from goodfetch_collie_client import BrowserName as BrowserName
from goodfetch_collie_client import BrowserSessionClass as BrowserSessionClass
from goodfetch_collie_client import BrowserSettings as BrowserSettings
from goodfetch_collie_client import BuiltinModelProvider as BuiltinModelProvider
from goodfetch_collie_client import CallCancelledError as CallCancelledError
from goodfetch_collie_client import CapabilityUnavailableDetails as CapabilityUnavailableDetails
from goodfetch_collie_client import CapabilityUnavailableError as CapabilityUnavailableError
from goodfetch_collie_client import CDPCommandRefusedError as CDPCommandRefusedError
from goodfetch_collie_client import CDPEndpoint as CDPEndpoint
from goodfetch_collie_client import CDPSession as CDPSession
from goodfetch_collie_client import CDPURLSource as CDPURLSource
from goodfetch_collie_client import Checkpoint as Checkpoint
from goodfetch_collie_client import CheckpointInfo as CheckpointInfo
from goodfetch_collie_client import CheckpointSettings as CheckpointSettings
from goodfetch_collie_client import ClosedError as ClosedError
from goodfetch_collie_client import CodeSourceBinding as CodeSourceBinding
from goodfetch_collie_client import Completeness as Completeness
from goodfetch_collie_client import ConfirmationCategory as ConfirmationCategory
from goodfetch_collie_client import Connection as Connection
from goodfetch_collie_client import ConsoleMessage as ConsoleMessage
from goodfetch_collie_client import ConsoleMessageLocation as ConsoleMessageLocation
from goodfetch_collie_client import Cookie as Cookie
from goodfetch_collie_client import CookieByDomain as CookieByDomain
from goodfetch_collie_client import CookieByURL as CookieByURL
from goodfetch_collie_client import CookieHeaderSource as CookieHeaderSource
from goodfetch_collie_client import CookieInput as CookieInput
from goodfetch_collie_client import Count as Count
from goodfetch_collie_client import DeadlineExceededDetails as DeadlineExceededDetails
from goodfetch_collie_client import DeadlineExceededError as DeadlineExceededError
from goodfetch_collie_client import DeclaresEffects as DeclaresEffects
from goodfetch_collie_client import Dialog as Dialog
from goodfetch_collie_client import DialogHandler as DialogHandler
from goodfetch_collie_client import Download as Download
from goodfetch_collie_client import DownloadFailedError as DownloadFailedError
from goodfetch_collie_client import Drift as Drift
from goodfetch_collie_client import Effect as Effect
from goodfetch_collie_client import EffectDone as EffectDone
from goodfetch_collie_client import EffectKeyInvalidDetails as EffectKeyInvalidDetails
from goodfetch_collie_client import EffectKeyInvalidError as EffectKeyInvalidError
from goodfetch_collie_client import EffectKeyReusedDetails as EffectKeyReusedDetails
from goodfetch_collie_client import EffectKeyReusedError as EffectKeyReusedError
from goodfetch_collie_client import EffectNotConfirmedDetails as EffectNotConfirmedDetails
from goodfetch_collie_client import EffectNotConfirmedError as EffectNotConfirmedError
from goodfetch_collie_client import EffectOutputInvalidDetails as EffectOutputInvalidDetails
from goodfetch_collie_client import EffectOutputInvalidError as EffectOutputInvalidError
from goodfetch_collie_client import EffectOutputSettings as EffectOutputSettings
from goodfetch_collie_client import EffectReconciled as EffectReconciled
from goodfetch_collie_client import EffectResult as EffectResult
from goodfetch_collie_client import EffectSettings as EffectSettings
from goodfetch_collie_client import EffectSourceUnknown as EffectSourceUnknown
from goodfetch_collie_client import EffectStagingRefusedDetails as EffectStagingRefusedDetails
from goodfetch_collie_client import EffectStagingRefusedError as EffectStagingRefusedError
from goodfetch_collie_client import EffectUncertainDetails as EffectUncertainDetails
from goodfetch_collie_client import EffectUncertainError as EffectUncertainError
from goodfetch_collie_client import ElementReferences as ElementReferences
from goodfetch_collie_client import ElementState as ElementState
from goodfetch_collie_client import Ended as Ended
from goodfetch_collie_client import EndedBy as EndedBy
from goodfetch_collie_client import EngineAddress as EngineAddress
from goodfetch_collie_client import EngineChoice as EngineChoice
from goodfetch_collie_client import EngineInfo as EngineInfo
from goodfetch_collie_client import ErrorGroup as ErrorGroup
from goodfetch_collie_client import ErrorTarget as ErrorTarget
from goodfetch_collie_client import EventInfo as EventInfo
from goodfetch_collie_client import EventIterator as EventIterator
from goodfetch_collie_client import expect as expect
from goodfetch_collie_client import ExplainedSetting as ExplainedSetting
from goodfetch_collie_client import ExtractionIncompleteDetails as ExtractionIncompleteDetails
from goodfetch_collie_client import ExtractionIncompleteError as ExtractionIncompleteError
from goodfetch_collie_client import ExtractionInvalidDetails as ExtractionInvalidDetails
from goodfetch_collie_client import ExtractionInvalidError as ExtractionInvalidError
from goodfetch_collie_client import ExtractionIssue as ExtractionIssue
from goodfetch_collie_client import ExtractMode as ExtractMode
from goodfetch_collie_client import ExtractSettings as ExtractSettings
from goodfetch_collie_client import FileChooser as FileChooser
from goodfetch_collie_client import FilePayload as FilePayload
from goodfetch_collie_client import Finished as Finished
from goodfetch_collie_client import FrameLocator as FrameLocator
from goodfetch_collie_client import Geolocation as Geolocation
from goodfetch_collie_client import GoodfetchError as GoodfetchError
from goodfetch_collie_client import HandlerFailedDetails as HandlerFailedDetails
from goodfetch_collie_client import HeadersInput as HeadersInput
from goodfetch_collie_client import HTTPCredentials as HTTPCredentials
from goodfetch_collie_client import HumanPauseDeclinedDetails as HumanPauseDeclinedDetails
from goodfetch_collie_client import HumanPauseDeclinedError as HumanPauseDeclinedError
from goodfetch_collie_client import HumanPauseResult as HumanPauseResult
from goodfetch_collie_client import Identities as Identities
from goodfetch_collie_client import InputFiles as InputFiles
from goodfetch_collie_client import InstalledBrowserSource as InstalledBrowserSource
from goodfetch_collie_client import InternalError as InternalError
from goodfetch_collie_client import JSONObject as JSONObject
from goodfetch_collie_client import JSONSchema as JSONSchema
from goodfetch_collie_client import JSONValue as JSONValue
from goodfetch_collie_client import JSONValueWithSecrets as JSONValueWithSecrets
from goodfetch_collie_client import Keyboard as Keyboard
from goodfetch_collie_client import KeyValueStore as KeyValueStore
from goodfetch_collie_client import KeyValueStoreReader as KeyValueStoreReader
from goodfetch_collie_client import lend_to_browser_use as lend_to_browser_use
from goodfetch_collie_client import lend_to_playwright as lend_to_playwright
from goodfetch_collie_client import LendsPage as LendsPage
from goodfetch_collie_client import Lifecycle as Lifecycle
from goodfetch_collie_client import LimitKind as LimitKind
from goodfetch_collie_client import LimitPeriod as LimitPeriod
from goodfetch_collie_client import LimitRaiseRefusedDetails as LimitRaiseRefusedDetails
from goodfetch_collie_client import LimitRaiseRefusedError as LimitRaiseRefusedError
from goodfetch_collie_client import LimitReachedDetails as LimitReachedDetails
from goodfetch_collie_client import LimitReachedError as LimitReachedError
from goodfetch_collie_client import Limits as Limits
from goodfetch_collie_client import LimitScope as LimitScope
from goodfetch_collie_client import LimitSource as LimitSource
from goodfetch_collie_client import Listener as Listener
from goodfetch_collie_client import ListenerFailedDetails as ListenerFailedDetails
from goodfetch_collie_client import ListenerFailedError as ListenerFailedError
from goodfetch_collie_client import LoadState as LoadState
from goodfetch_collie_client import Locator as Locator
from goodfetch_collie_client import LocatorExpectations as LocatorExpectations
from goodfetch_collie_client import LocatorSettings as LocatorSettings
from goodfetch_collie_client import LoggedOutDetails as LoggedOutDetails
from goodfetch_collie_client import LoggedOutError as LoggedOutError
from goodfetch_collie_client import MissingField as MissingField
from goodfetch_collie_client import ModelAllowanceReachedDetails as ModelAllowanceReachedDetails
from goodfetch_collie_client import ModelAllowanceReachedError as ModelAllowanceReachedError
from goodfetch_collie_client import ModelAllowanceSettings as ModelAllowanceSettings
from goodfetch_collie_client import ModelCapabilityMissingDetails as ModelCapabilityMissingDetails
from goodfetch_collie_client import ModelCapabilityMissingError as ModelCapabilityMissingError
from goodfetch_collie_client import ModelContextExceededDetails as ModelContextExceededDetails
from goodfetch_collie_client import ModelContextExceededError as ModelContextExceededError
from goodfetch_collie_client import ModelDeclaration as ModelDeclaration
from goodfetch_collie_client import ModelDeclinedDetails as ModelDeclinedDetails
from goodfetch_collie_client import ModelDeclinedError as ModelDeclinedError
from goodfetch_collie_client import ModelError as ModelError
from goodfetch_collie_client import ModelFailureDetails as ModelFailureDetails
from goodfetch_collie_client import ModelFormat as ModelFormat
from goodfetch_collie_client import ModelFundsExhaustedError as ModelFundsExhaustedError
from goodfetch_collie_client import ModelHandlerFailedDetails as ModelHandlerFailedDetails
from goodfetch_collie_client import ModelHandlerFailedError as ModelHandlerFailedError
from goodfetch_collie_client import ModelImage as ModelImage
from goodfetch_collie_client import ModelMessage as ModelMessage
from goodfetch_collie_client import ModelNotConfiguredDetails as ModelNotConfiguredDetails
from goodfetch_collie_client import ModelNotConfiguredError as ModelNotConfiguredError
from goodfetch_collie_client import ModelNotFoundError as ModelNotFoundError
from goodfetch_collie_client import ModelOutputInvalidDetails as ModelOutputInvalidDetails
from goodfetch_collie_client import ModelOutputInvalidError as ModelOutputInvalidError
from goodfetch_collie_client import ModelPriceUnknownDetails as ModelPriceUnknownDetails
from goodfetch_collie_client import ModelPriceUnknownError as ModelPriceUnknownError
from goodfetch_collie_client import ModelProviderHandler as ModelProviderHandler
from goodfetch_collie_client import ModelProviderSettings as ModelProviderSettings
from goodfetch_collie_client import ModelReference as ModelReference
from goodfetch_collie_client import ModelRequest as ModelRequest
from goodfetch_collie_client import ModelRequestRejectedDetails as ModelRequestRejectedDetails
from goodfetch_collie_client import ModelRequestRejectedError as ModelRequestRejectedError
from goodfetch_collie_client import ModelResponse as ModelResponse
from goodfetch_collie_client import ModelTool as ModelTool
from goodfetch_collie_client import ModelToolCall as ModelToolCall
from goodfetch_collie_client import ModelToolChoice as ModelToolChoice
from goodfetch_collie_client import ModelUnauthorizedError as ModelUnauthorizedError
from goodfetch_collie_client import ModelUnavailableDetails as ModelUnavailableDetails
from goodfetch_collie_client import ModelUnavailableError as ModelUnavailableError
from goodfetch_collie_client import ModelUsage as ModelUsage
from goodfetch_collie_client import ModelUsageOf as ModelUsageOf
from goodfetch_collie_client import Modifier as Modifier
from goodfetch_collie_client import Money as Money
from goodfetch_collie_client import Mouse as Mouse
from goodfetch_collie_client import MouseButton as MouseButton
from goodfetch_collie_client import NavigationFailedError as NavigationFailedError
from goodfetch_collie_client import NavigationInterruptedError as NavigationInterruptedError
from goodfetch_collie_client import NetworkSettings as NetworkSettings
from goodfetch_collie_client import NetworkThrottle as NetworkThrottle
from goodfetch_collie_client import NotGiven as NotGiven
from goodfetch_collie_client import NothingDone as NothingDone
from goodfetch_collie_client import NotVerifiedDetails as NotVerifiedDetails
from goodfetch_collie_client import NotVerifiedError as NotVerifiedError
from goodfetch_collie_client import ObservationSettings as ObservationSettings
from goodfetch_collie_client import Observed as Observed
from goodfetch_collie_client import Page as Page
from goodfetch_collie_client import PageAISettings as PageAISettings
from goodfetch_collie_client import PageAPI as PageAPI
from goodfetch_collie_client import PageCheck as PageCheck
from goodfetch_collie_client import PageCheckFailedError as PageCheckFailedError
from goodfetch_collie_client import PageCrashedError as PageCrashedError
from goodfetch_collie_client import PageError as PageError
from goodfetch_collie_client import PageEvents as PageEvents
from goodfetch_collie_client import PageExpectations as PageExpectations
from goodfetch_collie_client import PageFunctionFailedError as PageFunctionFailedError
from goodfetch_collie_client import PageHandlerFailedError as PageHandlerFailedError
from goodfetch_collie_client import PageMatch as PageMatch
from goodfetch_collie_client import PageRequest as PageRequest
from goodfetch_collie_client import PageSettings as PageSettings
from goodfetch_collie_client import PageStorage as PageStorage
from goodfetch_collie_client import PageTimeoutSettings as PageTimeoutSettings
from goodfetch_collie_client import PageValueInvalidError as PageValueInvalidError
from goodfetch_collie_client import PageWaits as PageWaits
from goodfetch_collie_client import PausesForHuman as PausesForHuman
from goodfetch_collie_client import PerformedAction as PerformedAction
from goodfetch_collie_client import Permission as Permission
from goodfetch_collie_client import PlanDriftDetails as PlanDriftDetails
from goodfetch_collie_client import PlanDriftError as PlanDriftError
from goodfetch_collie_client import PlanInvalidDetails as PlanInvalidDetails
from goodfetch_collie_client import PlanInvalidError as PlanInvalidError
from goodfetch_collie_client import PlanReport as PlanReport
from goodfetch_collie_client import PlanSettings as PlanSettings
from goodfetch_collie_client import PlanStepMissingDetails as PlanStepMissingDetails
from goodfetch_collie_client import PlanStepMissingError as PlanStepMissingError
from goodfetch_collie_client import PlaywrightChromium as PlaywrightChromium
from goodfetch_collie_client import PluginRun as PluginRun
from goodfetch_collie_client import PolitenessSharing as PolitenessSharing
from goodfetch_collie_client import Position as Position
from goodfetch_collie_client import ProfileDirectorySource as ProfileDirectorySource
from goodfetch_collie_client import ProgramError as ProgramError
from goodfetch_collie_client import ProxyAnswer as ProxyAnswer
from goodfetch_collie_client import ProxyHandlerFailedError as ProxyHandlerFailedError
from goodfetch_collie_client import ProxyPolitenessSettings as ProxyPolitenessSettings
from goodfetch_collie_client import ProxyProviderHandler as ProxyProviderHandler
from goodfetch_collie_client import ProxyRequest as ProxyRequest
from goodfetch_collie_client import ProxySettings as ProxySettings
from goodfetch_collie_client import RawCall as RawCall
from goodfetch_collie_client import Recording as Recording
from goodfetch_collie_client import RecordingInvalidDetails as RecordingInvalidDetails
from goodfetch_collie_client import RecordingInvalidError as RecordingInvalidError
from goodfetch_collie_client import RecordingMiss as RecordingMiss
from goodfetch_collie_client import RecordingReport as RecordingReport
from goodfetch_collie_client import RecoveryStep as RecoveryStep
from goodfetch_collie_client import ReferenceBinding as ReferenceBinding
from goodfetch_collie_client import RefusedError as RefusedError
from goodfetch_collie_client import RemovedError as RemovedError
from goodfetch_collie_client import Reported as Reported
from goodfetch_collie_client import ReportedTokens as ReportedTokens
from goodfetch_collie_client import Request as Request
from goodfetch_collie_client import ResolveSecret as ResolveSecret
from goodfetch_collie_client import ResourceType as ResourceType
from goodfetch_collie_client import Response as Response
from goodfetch_collie_client import ResponseExpectations as ResponseExpectations
from goodfetch_collie_client import ResponseNotJSONError as ResponseNotJSONError
from goodfetch_collie_client import ResponseNotRecordedDetails as ResponseNotRecordedDetails
from goodfetch_collie_client import ResponseNotRecordedError as ResponseNotRecordedError
from goodfetch_collie_client import Route as Route
from goodfetch_collie_client import RouteAlreadyHandledError as RouteAlreadyHandledError
from goodfetch_collie_client import RouteHandler as RouteHandler
from goodfetch_collie_client import Run as Run
from goodfetch_collie_client import RunAISettings as RunAISettings
from goodfetch_collie_client import RunConflictDetails as RunConflictDetails
from goodfetch_collie_client import RunConflictError as RunConflictError
from goodfetch_collie_client import RunControl as RunControl
from goodfetch_collie_client import RunEndedDetails as RunEndedDetails
from goodfetch_collie_client import RunEndedError as RunEndedError
from goodfetch_collie_client import RunEvents as RunEvents
from goodfetch_collie_client import RunIdDetails as RunIdDetails
from goodfetch_collie_client import RunIdInvalidError as RunIdInvalidError
from goodfetch_collie_client import RunInputDifference as RunInputDifference
from goodfetch_collie_client import RunInterruptedError as RunInterruptedError
from goodfetch_collie_client import RunInUseDetails as RunInUseDetails
from goodfetch_collie_client import RunInUseError as RunInUseError
from goodfetch_collie_client import RunLayerOrigin as RunLayerOrigin
from goodfetch_collie_client import RunningBrowserSource as RunningBrowserSource
from goodfetch_collie_client import RunNotFoundError as RunNotFoundError
from goodfetch_collie_client import RunProgress as RunProgress
from goodfetch_collie_client import RunReport as RunReport
from goodfetch_collie_client import RunSettings as RunSettings
from goodfetch_collie_client import RunStateUnavailableDetails as RunStateUnavailableDetails
from goodfetch_collie_client import RunStateUnavailableError as RunStateUnavailableError
from goodfetch_collie_client import RunStopReason as RunStopReason
from goodfetch_collie_client import RunTrace as RunTrace
from goodfetch_collie_client import RunUsage as RunUsage
from goodfetch_collie_client import RunView as RunView
from goodfetch_collie_client import RunWarning as RunWarning
from goodfetch_collie_client import SameSite as SameSite
from goodfetch_collie_client import SavedIdentity as SavedIdentity
from goodfetch_collie_client import SavedIdentitySource as SavedIdentitySource
from goodfetch_collie_client import SchemaIssue as SchemaIssue
from goodfetch_collie_client import SchemaValidatorFailedDetails as SchemaValidatorFailedDetails
from goodfetch_collie_client import SchemaValidatorFailedError as SchemaValidatorFailedError
from goodfetch_collie_client import SdkHandlerFailedDetails as SdkHandlerFailedDetails
from goodfetch_collie_client import secret as secret
from goodfetch_collie_client import SecretBinding as SecretBinding
from goodfetch_collie_client import SecretDomainNotAllowedDetails as SecretDomainNotAllowedDetails
from goodfetch_collie_client import SecretDomainNotAllowedError as SecretDomainNotAllowedError
from goodfetch_collie_client import SecretInputRefusedDetails as SecretInputRefusedDetails
from goodfetch_collie_client import SecretInputRefusedError as SecretInputRefusedError
from goodfetch_collie_client import SecretPlaceholder as SecretPlaceholder
from goodfetch_collie_client import SecretReference as SecretReference
from goodfetch_collie_client import SecretRequest as SecretRequest
from goodfetch_collie_client import SecretsSettings as SecretsSettings
from goodfetch_collie_client import SecretUnavailableDetails as SecretUnavailableDetails
from goodfetch_collie_client import SecretUnavailableError as SecretUnavailableError
from goodfetch_collie_client import SecretUnboundDetails as SecretUnboundDetails
from goodfetch_collie_client import SecretUnboundError as SecretUnboundError
from goodfetch_collie_client import SelectorInvalidError as SelectorInvalidError
from goodfetch_collie_client import Session as Session
from goodfetch_collie_client import SessionLoginSettings as SessionLoginSettings
from goodfetch_collie_client import SessionNotSavableDetails as SessionNotSavableDetails
from goodfetch_collie_client import SessionNotSavableError as SessionNotSavableError
from goodfetch_collie_client import SessionRecoverySettings as SessionRecoverySettings
from goodfetch_collie_client import SessionReservedError as SessionReservedError
from goodfetch_collie_client import SessionScopeLoginSettings as SessionScopeLoginSettings
from goodfetch_collie_client import SessionSettings as SessionSettings
from goodfetch_collie_client import SessionSource as SessionSource
from goodfetch_collie_client import SessionStateAPI as SessionStateAPI
from goodfetch_collie_client import SettingInvalidDetails as SettingInvalidDetails
from goodfetch_collie_client import SettingInvalidError as SettingInvalidError
from goodfetch_collie_client import SettingLockedDetails as SettingLockedDetails
from goodfetch_collie_client import SettingLockedError as SettingLockedError
from goodfetch_collie_client import SettingNotApplicableDetails as SettingNotApplicableDetails
from goodfetch_collie_client import SettingNotApplicableError as SettingNotApplicableError
from goodfetch_collie_client import SettingScope as SettingScope
from goodfetch_collie_client import SettingsLayer as SettingsLayer
from goodfetch_collie_client import SettingUnknownDetails as SettingUnknownDetails
from goodfetch_collie_client import SettingUnknownError as SettingUnknownError
from goodfetch_collie_client import StartedAs as StartedAs
from goodfetch_collie_client import StateFileSource as StateFileSource
from goodfetch_collie_client import StepReport as StepReport
from goodfetch_collie_client import StepResult as StepResult
from goodfetch_collie_client import StepSource as StepSource
from goodfetch_collie_client import Stopped as Stopped
from goodfetch_collie_client import StoredBytes as StoredBytes
from goodfetch_collie_client import StructuredOutputStrategy as StructuredOutputStrategy
from goodfetch_collie_client import Subscription as Subscription
from goodfetch_collie_client import SuccessCheck as SuccessCheck
from goodfetch_collie_client import SuccessCheckFailedDetails as SuccessCheckFailedDetails
from goodfetch_collie_client import SuccessCheckFailedError as SuccessCheckFailedError
from goodfetch_collie_client import SuccessCheckThrewDetails as SuccessCheckThrewDetails
from goodfetch_collie_client import SuccessCheckThrewError as SuccessCheckThrewError
from goodfetch_collie_client import SyncSuccessCheck as SyncSuccessCheck
from goodfetch_collie_client import TargetNotFoundDetails as TargetNotFoundDetails
from goodfetch_collie_client import TargetNotFoundError as TargetNotFoundError
from goodfetch_collie_client import TargetNotUniqueDetails as TargetNotUniqueDetails
from goodfetch_collie_client import TargetNotUniqueError as TargetNotUniqueError
from goodfetch_collie_client import TargetUnsupportedError as TargetUnsupportedError
from goodfetch_collie_client import TextExpected as TextExpected
from goodfetch_collie_client import TextMatch as TextMatch
from goodfetch_collie_client import Timeout as Timeout
from goodfetch_collie_client import TokenCounts as TokenCounts
from goodfetch_collie_client import Touchscreen as Touchscreen
from goodfetch_collie_client import UnavailableError as UnavailableError
from goodfetch_collie_client import UncertainEffect as UncertainEffect
from goodfetch_collie_client import UncertainWhy as UncertainWhy
from goodfetch_collie_client import URLMatcher as URLMatcher
from goodfetch_collie_client import ValueInvalidDetails as ValueInvalidDetails
from goodfetch_collie_client import ValueInvalidError as ValueInvalidError
from goodfetch_collie_client import VariablesMismatchDetails as VariablesMismatchDetails
from goodfetch_collie_client import VariablesMismatchError as VariablesMismatchError
from goodfetch_collie_client import Verdict as Verdict
from goodfetch_collie_client import VerdictReason as VerdictReason
from goodfetch_collie_client import Viewport as Viewport
from goodfetch_collie_client import WaitingPause as WaitingPause
from goodfetch_collie_client import WeakenedSetting as WeakenedSetting
from goodfetch_collie_client import WebError as WebError
from goodfetch_collie_client import WebSocket as WebSocket
from goodfetch_collie_client import WebSocketFrame as WebSocketFrame
from goodfetch_collie_client import WebStorage as WebStorage
from goodfetch_collie_client import WebStorageItem as WebStorageItem
from goodfetch_collie_client import World as World
from goodfetch_foxhound._connection import (
    AsyncCrawlRun as AsyncCrawlRun,
    AsyncCrawlRunView as AsyncCrawlRunView,
    AsyncFoxhound as AsyncFoxhound,
    CrawlRun as CrawlRun,
    CrawlRunView as CrawlRunView,
    Foxhound as Foxhound,
)
from goodfetch_foxhound._crawl import (
    AsyncCrawl as AsyncCrawl,
    AsyncCrawlContext as AsyncCrawlContext,
    AsyncHandler as AsyncHandler,
    AttemptEndReason as AttemptEndReason,
    AttemptEndedError as AttemptEndedError,
    BrowserRequiredError as BrowserRequiredError,
    Crawl as Crawl,
    CrawlContext as CrawlContext,
    CrawlRequest as CrawlRequest,
    EnqueueAdded as EnqueueAdded,
    EnqueueNotAdded as EnqueueNotAdded,
    Enqueued as Enqueued,
    EnqueuedLinks as EnqueuedLinks,
    FetchTier as FetchTier,
    Handler as Handler,
    HandlerLostError as HandlerLostError,
    LabelDuplicateError as LabelDuplicateError,
    LabelUnhandledError as LabelUnhandledError,
    LabelUnknownError as LabelUnknownError,
    LinkInfo as LinkInfo,
    NotAddedReason as NotAddedReason,
    ResponseNotOKError as ResponseNotOKError,
    ResponseUnavailableError as ResponseUnavailableError,
    RetryRequestedError as RetryRequestedError,
    Seed as Seed,
    SitePreferencesRefusedError as SitePreferencesRefusedError,
    SitemapSeed as SitemapSeed,
    URLPattern as URLPattern,
    seed as seed,
    sitemap as sitemap,
)
from goodfetch_foxhound._data import (
    Dataset as Dataset,
    Label as Label,
    PayloadInvalidDetails as PayloadInvalidDetails,
    PayloadInvalidError as PayloadInvalidError,
    PayloadSchemaChangedDetails as PayloadSchemaChangedDetails,
    PayloadSchemaChangedError as PayloadSchemaChangedError,
)
from goodfetch_foxhound._hooks import (
    AfterResponseContext as AfterResponseContext,
    AfterResponseHook as AfterResponseHook,
    AsyncAfterResponseContext as AsyncAfterResponseContext,
    AsyncAfterResponseHook as AsyncAfterResponseHook,
    AsyncBeforeRequestContext as AsyncBeforeRequestContext,
    AsyncBeforeRequestHook as AsyncBeforeRequestHook,
    AsyncFetchBackendHandler as AsyncFetchBackendHandler,
    AsyncHooks as AsyncHooks,
    AsyncPageStageContext as AsyncPageStageContext,
    AsyncPageStageHook as AsyncPageStageHook,
    AsyncRecordCommitContext as AsyncRecordCommitContext,
    AsyncRecordCommitHook as AsyncRecordCommitHook,
    AsyncSinkHandler as AsyncSinkHandler,
    BackendRequest as BackendRequest,
    BackendResponse as BackendResponse,
    BeforeRequestContext as BeforeRequestContext,
    BeforeRequestHook as BeforeRequestHook,
    FetchBackendHandler as FetchBackendHandler,
    FetchHandlerFailedError as FetchHandlerFailedError,
    HeaderForbiddenDetails as HeaderForbiddenDetails,
    HeaderForbiddenError as HeaderForbiddenError,
    HookDuplicateDetails as HookDuplicateDetails,
    HookDuplicateError as HookDuplicateError,
    HookFailedDetails as HookFailedDetails,
    HookFailedError as HookFailedError,
    HookIneffectiveDetails as HookIneffectiveDetails,
    HookIneffectiveError as HookIneffectiveError,
    HookStage as HookStage,
    PageStageContext as PageStageContext,
    PageStageHook as PageStageHook,
    RecordCommitContext as RecordCommitContext,
    RecordCommitHook as RecordCommitHook,
    ReportedHookStage as ReportedHookStage,
    RequestChange as RequestChange,
    ResponseChange as ResponseChange,
    SinkDelivery as SinkDelivery,
    SinkHandler as SinkHandler,
    SinkHandlerFailedError as SinkHandlerFailedError,
    SyncHooks as SyncHooks,
)
from goodfetch_foxhound._outcomes import (
    CrawlRequestBlocked as CrawlRequestBlocked,
    CrawlRequestFailed as CrawlRequestFailed,
    CrawlRequestOutcome as CrawlRequestOutcome,
    CrawlRequestOutcomeUnknown as CrawlRequestOutcomeUnknown,
    CrawlRequestRef as CrawlRequestRef,
    CrawlRequestSkipped as CrawlRequestSkipped,
    CrawlRequestSucceeded as CrawlRequestSucceeded,
    ErrorCode as ErrorCode,
    HandlerFailedError as HandlerFailedError,
    KnownErrorCode as KnownErrorCode,
    RecordCounts as RecordCounts,
    SkipReason as SkipReason,
)
from goodfetch_foxhound._output import (
    AsyncExport as AsyncExport,
    DatasetDetails as DatasetDetails,
    DatasetDuplicateError as DatasetDuplicateError,
    DatasetKeyInvalidDetails as DatasetKeyInvalidDetails,
    DatasetKeyInvalidError as DatasetKeyInvalidError,
    DatasetRecord as DatasetRecord,
    DatasetUnknownDetails as DatasetUnknownDetails,
    DatasetUnknownError as DatasetUnknownError,
    Export as Export,
    ExportFormat as ExportFormat,
    RecordInvalidDetails as RecordInvalidDetails,
    RecordInvalidError as RecordInvalidError,
    RejectedRecord as RejectedRecord,
)
from goodfetch_foxhound._politeness import (
    BackoffSettings as BackoffSettings,
    DomainPolitenessSettings as DomainPolitenessSettings,
    IPPolitenessSettings as IPPolitenessSettings,
    PolitenessSettings as PolitenessSettings,
    SitePreferencesSettings as SitePreferencesSettings,
)
from goodfetch_foxhound._run import (
    CrawlConcurrency as CrawlConcurrency,
    CrawlRequestCounts as CrawlRequestCounts,
    CrawlRequestTotals as CrawlRequestTotals,
    CrawlRunProgress as CrawlRunProgress,
    CrawlRunReport as CrawlRunReport,
    CrawlUncertainEffect as CrawlUncertainEffect,
    DatasetTotals as DatasetTotals,
    GovernorState as GovernorState,
    HandlerConcurrency as HandlerConcurrency,
    InFlightRequest as InFlightRequest,
    LearnedRule as LearnedRule,
    PermitWait as PermitWait,
    SinkDeliveryStatus as SinkDeliveryStatus,
    SitemapTotals as SitemapTotals,
    UnreadSitemap as UnreadSitemap,
    WaitGroup as WaitGroup,
    WaitReason as WaitReason,
    WaitingDomain as WaitingDomain,
)
from goodfetch_foxhound._settings import (
    CrawlHandlerSettings as CrawlHandlerSettings,
    CrawlRetrySettings as CrawlRetrySettings,
    CrawlRunSettings as CrawlRunSettings,
    CrawlScopeSettings as CrawlScopeSettings,
    CrawlScopeStrategy as CrawlScopeStrategy,
    CrawlSettings as CrawlSettings,
    FetchSettings as FetchSettings,
    LabelCrawlSettings as LabelCrawlSettings,
    LabelFetchSettings as LabelFetchSettings,
    LabelSettings as LabelSettings,
    LabelTimeoutSettings as LabelTimeoutSettings,
    PayloadSettings as PayloadSettings,
    RunTimeoutSettings as RunTimeoutSettings,
    SettingKey as SettingKey,
    Settings as Settings,
    SinkSettings as SinkSettings,
    SinkWriteMode as SinkWriteMode,
    TimeoutSettings as TimeoutSettings,
)
