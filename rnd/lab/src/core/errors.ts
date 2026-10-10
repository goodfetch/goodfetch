// Typed lab errors: attempt status derives from the code, never from message text.
export const errorCodes = ['E_ADAPTER_TIMEOUT', 'E_ADAPTER_EXIT', 'E_ADAPTER_CLOSED', 'E_ADAPTER_ERROR', 'E_PROTOCOL', 'E_OUTPUT_LIMIT', 'E_SIBLING_ABORT',
  'E_STARTUP', 'E_RESTART_LIMIT', 'E_INVALID_OUTPUT', 'E_ORACLE_MISMATCH', 'E_CAPACITY', 'E_DEADLINE', 'E_CAPABILITY', 'E_HOST_GATE', 'E_FIXTURE', 'E_LANE_EXCEPTION'] as const;
export type ErrorCode = typeof errorCodes[number];

export class LabError extends Error {
  readonly code: ErrorCode;
  constructor(code: ErrorCode, message: string, options?: { cause?: unknown }) { super(message, options); this.code = code; this.name = 'LabError'; }
}

export function errorCode(error: unknown): ErrorCode {
  return error instanceof LabError ? error.code : 'E_ADAPTER_ERROR';
}

export type AttemptStatus = 'verified' | 'failed' | 'timeout' | 'crashed' | 'aborted' | 'unsupported' | 'rejected' | 'unavailable' | 'skipped-gate';
const statusByCode: Partial<Record<ErrorCode, AttemptStatus>> = {
  E_ADAPTER_TIMEOUT: 'timeout', E_SIBLING_ABORT: 'aborted', E_ADAPTER_EXIT: 'crashed', E_ADAPTER_CLOSED: 'crashed', E_PROTOCOL: 'crashed',
  E_OUTPUT_LIMIT: 'crashed', E_STARTUP: 'crashed', E_RESTART_LIMIT: 'crashed', E_CAPACITY: 'rejected', E_DEADLINE: 'rejected', E_CAPABILITY: 'unsupported'
};
export function statusFor(code: ErrorCode): AttemptStatus { return statusByCode[code] ?? 'failed'; }
