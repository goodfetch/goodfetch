// Each state of a crawl run's request log (TESTING.md, Components; spec
// #593, story 219): the log with every kind of outcome, a request whose
// attempts failed with a TypeScript, a Python and a Rust handler's
// exception, a failed Hook with its stage, Hook, Label and wrapped code,
// and the filters that ask the engine for an outcome, a Label and an Error
// code. Foxhound's messages join the Console's before each story, as
// Foxhound's build entry adds them.
import type { AttemptError, CrawlRequestOutcome } from "@goodfetch/foxhound/generated";
import { i18n } from "@goodfetch/console/i18n";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { foxhoundMessages } from "#messages.ts";
import { LogControls, OutcomeLog } from "./request-log.tsx";

/** Loads Foxhound's messages in the Console's locale. */
async function withFoxhoundMessages(): Promise<Record<string, never>> {
  const { locale } = i18n;
  if (locale === "en" || locale === "en-XA" || locale === "ar-XB") {
    i18n.load(locale, await foxhoundMessages(locale));
    i18n.activate(locale);
  }
  return {};
}

const ENDED_AT = "2026-10-26T09:00:00Z";

/** The request at `position`, of the Label `label`. */
function request(position: number, label: string): CrawlRequestOutcome["request"] {
  return { id: String(position), url: `https://shop.test/${String(position)}`, label, depth: 1 };
}

/** A request at `position` that failed on each of `errors`. */
function failed(position: number, errors: readonly AttemptError[]): CrawlRequestOutcome {
  const last = errors.at(-1) ?? { attempt: 1, code: "handler_failed", message: "" };
  return {
    position,
    request: request(position, "product"),
    attempts: errors.length,
    errors: [...errors],
    ended_at: ENDED_AT,
    kind: "failed",
    error: last,
  };
}

const meta: Meta<typeof OutcomeLog> = {
  title: "Request log",
  component: OutcomeLog,
  loaders: [withFoxhoundMessages],
};

export default meta;

type Story = StoryObj<typeof OutcomeLog>;

/** Every kind of outcome, one retried before it succeeded. */
export const EveryOutcome: Story = {
  args: {
    outcomes: [
      {
        position: 1,
        request: request(1, "list"),
        attempts: 2,
        errors: [{ attempt: 1, code: "response_unavailable", message: "the site answered 503" }],
        ended_at: ENDED_AT,
        kind: "succeeded",
        records: [{ dataset: "products", committed: 1, dropped: 0, rejected: 0 }],
      },
      {
        position: 2,
        request: request(2, "product"),
        attempts: 1,
        errors: [],
        ended_at: ENDED_AT,
        kind: "skipped",
        reason: "robots_txt",
      },
      {
        position: 3,
        request: request(3, "product"),
        attempts: 1,
        errors: [],
        ended_at: ENDED_AT,
        kind: "blocked",
        cause: "challenge",
      },
    ],
  },
};

/** A handler's exception in each language: TypeScript and Python with their stack, Rust with none. */
export const HandlerExceptions: Story = {
  args: {
    outcomes: [
      failed(4, [
        {
          attempt: 1,
          code: "handler_failed",
          message: "the handler threw TypeError: Cannot read properties of undefined",
          details: {
            exception_type: "TypeError",
            exception_message: "Cannot read properties of undefined (reading 'price')",
            exception_stack:
              "TypeError: Cannot read properties of undefined (reading 'price')\n    at handle (/app/crawl.ts:12:18)",
          },
        },
        {
          attempt: 2,
          code: "handler_failed",
          message: "the handler raised KeyError: 'price'",
          details: {
            exception_type: "KeyError",
            exception_message: "'price'",
            exception_stack:
              "Traceback (most recent call last):\n  File \"/app/crawl.py\", line 9, in handle\nKeyError: 'price'",
          },
        },
        {
          attempt: 3,
          code: "handler_failed",
          message: "the handler returned BoxError: invalid digit found in string",
          details: {
            exception_type: "BoxError",
            exception_message: "invalid digit found in string",
            exception_stack: "",
          },
        },
      ]),
    ],
  },
};

/** A Hook's failure: a Goodfetch code it raised, wrapped, and a panic at a lifecycle stage. */
export const HookFailures: Story = {
  args: {
    outcomes: [
      failed(5, [
        {
          attempt: 1,
          code: "hook_failed",
          message: "the Hook add_token raised header_forbidden",
          details: {
            stage: "before_request",
            hook: "add_token",
            label: "product",
            wrapped_code: "header_forbidden",
            exception_type: "GoodfetchError",
            exception_message: "the header Cookie is the engine's own",
            exception_stack: "GoodfetchError: the header Cookie is the engine's own",
          },
        },
        {
          attempt: 2,
          code: "hook_failed",
          message: "the Hook on_start panicked",
          details: {
            stage: "on_start",
            hook: "on_start",
            label: null,
            wrapped_code: null,
            exception_type: "panic",
            exception_message: "index out of bounds",
            exception_stack: "",
          },
        },
      ]),
    ],
  },
};

/** The filters that ask the engine for outcomes of a kind, a Label and an Error code. */
export const Filters: Story = {
  render: () => (
    <LogControls
      filters={{ kinds: ["failed"], labels: ["product"], codes: ["handler_failed"] }}
      onApply={(): void => {}}
    />
  ),
};
