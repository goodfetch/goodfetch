// The handlers a library may never install in its program's process (ADR
// 0078): a signal's, an exit's, an uncaught
// exception's, an unhandled rejection's or a warning's, the events Collie's
// guest program counts.

/** How many listeners the process has for each such event. */
export function processHandlers(): Record<string, number> {
  return Object.fromEntries(
    [
      "SIGINT",
      "SIGTERM",
      "SIGHUP",
      "SIGBREAK",
      "exit",
      "beforeExit",
      "uncaughtException",
      "unhandledRejection",
      "warning",
    ].map((event) => [event, process.listenerCount(event)]),
  );
}
