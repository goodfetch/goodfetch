// A host logger for the SDK tests that play a script on the Scripted engine:
// the engine's stderr lines reach the host's logger as warnings, and a failed
// play writes one, so a test that saw no warning saw its script play through.
import type { Logger } from "@goodfetch/collie-client";

/** A host logger that keeps each warning, where the engine's stderr lines go. */
export function recordingLogger(): { readonly logger: Logger; readonly warned: string[] } {
  const warned: string[] = [];
  const logger: Logger = {
    // It asks for none of the engine's log events: `hello` says `off`, as the scripts expect.
    level: "off",
    debug: () => {},
    info: () => {},
    warn: (message) => {
      warned.push(message);
    },
    error: () => {},
  };
  return { logger, warned };
}
