// A failure as the Console shows it (ADRs 0073 and 0079): the catalog's
// message with the operation first, every field the engine sent, as data
// under `translate="no"`, the call log, and the program's own exception for
// each code that always names it; a code the catalog does not know shows
// the engine's English message.
import { errorOfReply } from "@goodfetch/collie-client/browser";
import { collieErrorClasses } from "@goodfetch/collie-client/generated";
import { expect, test } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import { ConsoleI18n } from "#i18n.tsx";
import { ErrorMessage } from "./error-message.tsx";

/** The error the Console's Client raises for an engine's failure with `code` and `details`. */
function engineError(code: string, details: Readonly<Record<string, unknown>> = {}): unknown {
  return errorOfReply(
    {
      code: -32001,
      message: `the engine's English message for ${code}`,
      data: { code, retryable: false, ...details },
    },
    collieErrorClasses,
  );
}

/** Renders the failure `error` as the Console shows it. */
async function shown(error: unknown): Promise<HTMLElement> {
  const { container } = await render(
    <ConsoleI18n>
      <ErrorMessage error={error} />
    </ConsoleI18n>,
  );
  return container;
}

/** Whether the text `value` is shown inside `translate="no"`. */
function untranslated(value: string): boolean {
  return page.getByText(value, { exact: true }).element().closest('[translate="no"]') !== null;
}

test(
  "shows the catalog's message, then each field as data, then the call log",
  { tags: ["story:12-133"] },
  async () => {
    const container = await shown(
      engineError("deadline_exceeded", {
        operation: "locator.click",
        target: "page.getByRole('button', { name: 'Next' })",
        url: "https://shop.test/cart",
        observed: "the button is disabled",
        limit: { key: "timeout.action", value: 30_000, layer: "user_configuration" },
        fix: "raise `timeout.action`",
        call_log: ["waiting for the button", "the button is still disabled"],
      }),
    );

    expect(container.textContent).toMatch(
      /^locator\.click: The call's deadline passed before what it waited for came\.Targetpage\.getByRole\('button', \{ name: 'Next' \}\)URLhttps:\/\/shop\.test\/cartObservedthe button is disabledLimittimeout\.action is .+, set by User configurationFixraise `timeout\.action`Error codedeadline_exceededCall logwaiting for the buttonthe button is still disabled$/u,
    );
    expect(
      [
        "locator.click",
        "page.getByRole('button', { name: 'Next' })",
        "https://shop.test/cart",
        "the button is disabled",
        "timeout.action",
        "raise `timeout.action`",
        "deadline_exceeded",
        "waiting for the button",
      ].filter((value) => !untranslated(value)),
    ).toEqual([]);
    expect(container.querySelector("data")?.getAttribute("value")).toBe("30000");
  },
);

test.each([
  "handler_failed",
  "hook_failed",
  "model_handler_failed",
  "agent_tool_failed",
  "success_check_threw",
  "schema_validator_failed",
])("names the program's own exception for %s", { tags: ["story:12-133"] }, async (code) => {
  const container = await shown(
    engineError(code, {
      exception_type: "TypeError",
      exception_message: "Cannot read properties of undefined (reading 'price')",
    }),
  );

  expect(container.textContent).toContain(
    "The program's own exception: TypeError, Cannot read properties of undefined (reading 'price')",
  );
  expect(
    ["TypeError", "Cannot read properties of undefined (reading 'price')"].filter(
      (value) => !untranslated(value),
    ),
  ).toEqual([]);
});

test(
  "shows the engine's English message for a code the catalog does not know",
  { tags: ["story:12-133"] },
  async () => {
    const container = await shown(engineError("no_such_code", { operation: "run.list" }));

    expect(container.textContent).toBe("the engine's English message for no_such_code");
    expect(untranslated("the engine's English message for no_such_code")).toBe(true);
  },
);
