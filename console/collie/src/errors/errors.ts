// Each Error code's message from the Console's catalog (spec #592's
// `messages`; ADRs 0073 and 0079): what failed, in the catalog's words,
// beside the error's fields as the engine sent them (operation, target, URL,
// observed, `limit` with its setting, value and layer, `fix`, call log),
// which are data and never translated. A code the catalog does not know
// keeps the engine's English message, which the engine renders from the
// same fields. The program's own exception (`handler_failed`, `hook_failed`,
// each `<kind>_handler_failed`, `agent_tool_failed`, `success_check_threw`
// and `schema_validator_failed`) is always named, by its type and message
// (ADRs 0079, 0082, 0100 and 0104); when the details do not carry them, the
// engine's message, which names it, is kept instead.
import type { I18n, MessageDescriptor } from "@lingui/core";
import { msg } from "@lingui/core/macro";
import { GoodfetchError, type EndingError } from "@goodfetch/collie-client/generated";
import { isSettingsLayer, LAYERS } from "#settings-layers.ts";

/** What the catalog says failed, for each Error code it knows. */
const SUMMARIES: Readonly<Record<string, MessageDescriptor>> = {
  address_family_unavailable: msg`The host has no address in an IP family the Session can reach it from.`,
  agent_tool_failed: msg`An Agent tool raised its own exception.`,
  artifact_download_failed: msg`A pinned download the call needed failed.`,
  artifact_unavailable_offline: msg`The offline setting refused a pinned download the call needed.`,
  blocked: msg`The site refused the page with a Block cause the engine cannot get past.`,
  body_not_found: msg`The run's folder holds no body of this ID.`,
  body_unavailable: msg`The browser no longer holds the response's body.`,
  browser_host_outdated: msg`The Browser host is of a version this engine cannot drive.`,
  browser_launch_failed: msg`The browser did not start or did not open the page.`,
  browser_not_installed: msg`The browser the launch needs is not installed.`,
  browser_sandbox_unavailable: msg`Chromium's sandbox cannot start on this host, so no browser was launched.`,
  call_cancelled: msg`The call was stopped before it ended.`,
  capability_unavailable: msg`This engine cannot provide what a setting asks for.`,
  cdp_url_not_allowed: msg`The address of the Session source is not allowed.`,
  certificate_changed: msg`A pinned host presented a certificate with another key, so nothing was sent to it.`,
  certificate_invalid: msg`The host's certificate failed verification, so nothing was sent to it.`,
  closed: msg`The call was made on something its program had closed.`,
  connection_lost: msg`The Connection to the engine dropped before the call was answered, so whether it took effect is unknown.`,
  console_port_in_use: msg`The Console did not start: another program holds its port.`,
  daemon_draining: msg`The Daemon is stopping, so it takes no run.`,
  deadline_exceeded: msg`The call's deadline passed before what it waited for came.`,
  dialog_already_handled: msg`The dialog was answered already.`,
  egress_refused: msg`The run's egress policy refused the request before it connected.`,
  engine_exited: msg`The engine ended before the call was answered.`,
  engine_not_found: msg`No engine executable was found to start.`,
  engine_output_invalid: msg`The engine wrote something that is no protocol message, so the Connection ended.`,
  executable_blocked_by_smart_app_control: msg`Windows refused to start the executable.`,
  file_write_failed: msg`A file could not be written.`,
  format_version_unsupported: msg`A file is of a format version this engine cannot read.`,
  handler_failed: msg`The program's handler raised its own exception.`,
  header_value_invalid: msg`A header value held a line break or a NUL, so it was not sent.`,
  hook_failed: msg`A Hook raised its own exception.`,
  internal: msg`The engine met a bug of its own; please report it.`,
  limit_not_understood: msg`A newer engine wrote a Limit that may cover the run, which this engine cannot evaluate.`,
  limit_reached: msg`A Limit that covers the run has nothing left in its period.`,
  local_address_unavailable: msg`The Session's local addresses hold no address this host can send from.`,
  logged_out: msg`The page is a login page: the Session's login is lost.`,
  name_lookup_failed: msg`The engine's resolver got no usable answer for the host name.`,
  name_not_resolved: msg`No address was found for the host name.`,
  navigation_failed: msg`The navigation reached no document.`,
  navigation_interrupted: msg`Another navigation started before this one reached its document.`,
  not_a_daemon: msg`This engine is no Daemon.`,
  not_in_product: msg`This engine's Product does not carry what was asked.`,
  page_check_failed: msg`The page is neither the expected page nor the login page.`,
  page_crashed: msg`The page crashed.`,
  page_events_lost: msg`The engine fell too far behind the page's events and no longer reads them.`,
  page_function_failed: msg`A page function failed in the page.`,
  page_handler_failed: msg`A handler the program gave the page failed.`,
  page_memory_exceeded: msg`The page held more memory than its cap while memory was critical, so it was ended.`,
  page_value_invalid: msg`A page function's value is no JSON.`,
  path_not_writable: msg`The path cannot be written.`,
  path_unsafe: msg`A name or path would leave its folder or is no safe file name on every system.`,
  pattern_unsupported: msg`The engine cannot match this URL pattern.`,
  person_required: msg`Only a person can give this answer.`,
  policy_conflict: msg`The Operator policy changed since it was read, so nothing was written.`,
  protocol_version_unsupported: msg`The engine does not speak the protocol version the Client asked for.`,
  proxy_auth_failed: msg`The proxy refused the Session's credentials.`,
  proxy_provider_unavailable: msg`The Session's Proxy provider answered with a failure of its own, such as a spent quota or a lost sticky exit.`,
  proxy_relay_unavailable: msg`The engine could not open the local relay to the Session's proxy.`,
  proxy_unavailable: msg`The Session's proxy could not be reached.`,
  resources_exhausted: msg`The engine ran out of memory or storage for the run.`,
  response_limit_exceeded: msg`A response passed one of the run's limits on what the engine reads, so it was not read.`,
  response_truncated: msg`The response's body broke off before its end.`,
  route_already_handled: msg`The route's request was answered already.`,
  run_busy: msg`Another engine holds the run, and its owner did not take the action.`,
  run_conflict: msg`The Run ID names a run started with other inputs.`,
  run_ended: msg`The run has ended, so it takes no new work.`,
  run_id_invalid: msg`The Run ID is not one the engine takes.`,
  run_in_use: msg`Another engine writes this run.`,
  run_interrupted: msg`The run was interrupted; starting it again with its Run ID resumes it.`,
  run_key_mismatch: msg`The run's data key does not open on this machine.`,
  run_live: msg`The run is still live, so it cannot be discarded.`,
  run_not_found: msg`No run has this Run ID.`,
  run_not_owned: msg`This engine does not own the run.`,
  run_state_unavailable: msg`The run's state is missing, damaged or cannot be written.`,
  sample_refused: msg`The sample operation refused the text.`,
  schema_validator_failed: msg`A schema's own validator raised an exception.`,
  secret_domain_not_allowed: msg`A Secret's value would go to a domain outside its binding.`,
  secret_input_refused: msg`A Secret's value would be typed or sent where other code may read it, so it was not resolved.`,
  secret_unavailable: msg`The Secret provider gave no value.`,
  secret_unbound: msg`No scope binds the Secret placeholder.`,
  session_bound_to_machine: msg`The run is bound to the machine of its Session source, and this is another machine.`,
  setting_invalid: msg`A setting's value is invalid.`,
  setting_locked: msg`A lower settings layer locked this value.`,
  setting_not_applicable: msg`A setting was given where it cannot take effect.`,
  setting_unknown: msg`No Product knows this setting.`,
  site_preferences_refused: msg`The site's preferences refused the request.`,
  success_check_threw: msg`The Success check raised its own exception.`,
  target_not_unique: msg`The locator matched several elements, so the action did nothing.`,
  too_many_redirects: msg`The navigation was redirected too many times.`,
  upgrade_source_unsupported: msg`This release cannot upgrade files of that major.`,
  value_invalid: msg`The Key-value store refused the key or its value.`,
};

/** What the catalog says of a `<kind>_handler_failed` it has no message of its own for (ADR 0104). */
const SDK_HANDLER_FAILED = msg`A handler the program gave the engine raised its own exception.`;

/** The Error codes whose message always names the program's own exception, beside each `<kind>_handler_failed`. */
const PROGRAM_EXCEPTION_CODES: ReadonlySet<string> = new Set([
  "handler_failed",
  "hook_failed",
  "agent_tool_failed",
  "success_check_threw",
  "schema_validator_failed",
]);

/** The setting, value and layer an error's `limit` names (ADR 0079). */
export interface ErrorLimit {
  /** The setting's key. */
  readonly key: string;
  /** Its value, as JSON. */
  readonly value: unknown;
  /** The name of the layer that set it, in the catalog's words when it is a layer the catalog knows. */
  readonly layer: string | undefined;
}

/** The program's own exception, as its error's details carry it. */
export interface ProgramException {
  /** The exception's type name. */
  readonly type: string;
  /** Its message. */
  readonly message: string;
}

/** How the Console shows a failure: the catalog's message with the error's fields, or the English message it came with. */
export type ErrorDescription =
  | {
      readonly kind: "catalog";
      /** The Error code. */
      readonly code: string;
      /** What failed, in the catalog's words. */
      readonly summary: string;
      /** The operation that failed, as the catalog names it. */
      readonly operation: string | undefined;
      /** What the call acted on, in the Client's language. */
      readonly target: string | undefined;
      /** The URL it concerned. */
      readonly url: string | undefined;
      /** What was there instead. */
      readonly observed: string | undefined;
      /** The setting whose value bounded the call. */
      readonly limit: ErrorLimit | undefined;
      /** What would change the outcome, in the engine's words. */
      readonly fix: string | undefined;
      /** The engine's steps for the call. */
      readonly callLog: readonly string[];
      /** The program's own exception, for a code that always names it. */
      readonly exception: ProgramException | undefined;
    }
  | {
      readonly kind: "engine";
      /** The Error code, when the failure carries one. */
      readonly code: string | undefined;
      /** Its English message, as it came. */
      readonly message: string;
    };

/** Whether `code`'s message always names the program's own exception. */
function namesProgramException(code: string): boolean {
  return PROGRAM_EXCEPTION_CODES.has(code) || code.endsWith("_handler_failed");
}

/** What the catalog says failed for `code`, if it knows the code. */
function summaryOf(code: string): MessageDescriptor | undefined {
  if (Object.hasOwn(SUMMARIES, code)) {
    return SUMMARIES[code];
  }
  return code.endsWith("_handler_failed") ? SDK_HANDLER_FAILED : undefined;
}

/** `details[name]` when it is text. */
function text(details: Readonly<Record<string, unknown>>, name: string): string | undefined {
  const value = details[name];
  return typeof value === "string" ? value : undefined;
}

/** `details[name]` as text: as it is when it is text, else as JSON. */
function shown(details: Readonly<Record<string, unknown>>, name: string): string | undefined {
  const value = details[name];
  if (value === undefined || value === null) {
    return undefined;
  }
  return typeof value === "string" ? value : JSON.stringify(value);
}

/** The error's `limit`, with its layer named in the catalog's words. */
function limitOf(details: Readonly<Record<string, unknown>>, i18n: I18n): ErrorLimit | undefined {
  const limit = details["limit"];
  if (typeof limit !== "object" || limit === null || !("key" in limit)) {
    return undefined;
  }
  const { key } = limit;
  if (typeof key !== "string") {
    return undefined;
  }
  const value = "value" in limit ? limit.value : undefined;
  const layer = "layer" in limit && typeof limit.layer === "string" ? limit.layer : undefined;
  return {
    key,
    value,
    layer: layer !== undefined && isSettingsLayer(layer) ? i18n._(LAYERS[layer]) : layer,
  };
}

/** The error's call log: its steps, in order. */
function callLogOf(details: Readonly<Record<string, unknown>>): readonly string[] {
  const log = details["call_log"];
  return Array.isArray(log) ? log.filter((step) => typeof step === "string") : [];
}

/** The program's own exception the details name, if they name its type and message. */
function exceptionOf(details: Readonly<Record<string, unknown>>): ProgramException | undefined {
  const type = text(details, "exception_type");
  const message = text(details, "exception_message");
  return type === undefined || message === undefined ? undefined : { type, message };
}

/** What the catalog says failed for `code`, in its words; none when it does not know the code. */
export function catalogSummary(code: string, i18n: I18n): string | undefined {
  const summary = summaryOf(code);
  return summary === undefined ? undefined : i18n._(summary);
}

/** An Error code as it came: the code, its English message, its details and its fix. */
interface Coded {
  readonly code: string;
  readonly message: string;
  readonly details: Readonly<Record<string, unknown>>;
  readonly fix: string | undefined;
}

/** How the Console shows the coded failure `coded`, with the catalog of `i18n`. */
function describeCoded({ code, message, details, fix }: Coded, i18n: I18n): ErrorDescription {
  const summary = summaryOf(code);
  const exception = exceptionOf(details);
  if (summary === undefined || (namesProgramException(code) && exception === undefined)) {
    return { kind: "engine", code, message };
  }
  return {
    kind: "catalog",
    code,
    summary: i18n._(summary),
    operation: text(details, "operation"),
    target: shown(details, "target"),
    url: text(details, "url"),
    observed: shown(details, "observed"),
    limit: limitOf(details, i18n),
    fix,
    callLog: callLogOf(details),
    exception: namesProgramException(code) ? exception : undefined,
  };
}

/** How the Console shows `failure`, with the catalog of `i18n`. */
export function describeError(failure: unknown, i18n: I18n): ErrorDescription {
  if (!(failure instanceof GoodfetchError)) {
    return {
      kind: "engine",
      code: undefined,
      message: failure instanceof Error ? failure.message : String(failure),
    };
  }
  const { code, message, details, fix } = failure;
  return describeCoded({ code, message, details, fix }, i18n);
}

/** How the Console shows the error a Run report's ending raised (ADR 0116), which carries no fix. */
export function describeEnding(ending: EndingError, i18n: I18n): ErrorDescription {
  return describeCoded(
    { code: ending.code, message: ending.message, details: ending.details ?? {}, fix: undefined },
    i18n,
  );
}
