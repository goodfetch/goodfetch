// Each state of a failure as the Console shows it (TESTING.md, Components):
// the catalog's message with every field the engine sends, a program's own
// exception, a short refusal, and a code the catalog does not know, which
// keeps the engine's English message.
import {
  DeadlineExceededError,
  PersonRequiredError,
  ProgramError,
} from "@goodfetch/collie-client/generated";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { ErrorMessage } from "./error-message.tsx";

const meta: Meta<typeof ErrorMessage> = {
  title: "Error message",
  component: ErrorMessage,
};

export default meta;

type Story = StoryObj<typeof ErrorMessage>;

/** A deadline that passed, with every field. */
export const EveryField: Story = {
  args: {
    error: new DeadlineExceededError("locator.click: the deadline passed", {
      code: "deadline_exceeded",
      group: "deadline_exceeded",
      retryable: false,
      fix: "raise `timeout.action`, or wait for the button to be enabled",
      details: {
        operation: "locator.click",
        target: "page.getByRole('button', { name: 'Next' })",
        url: "https://shop.test/cart",
        observed: "the button is disabled",
        limit: { key: "timeout.action", value: 30_000, layer: "user_configuration" },
        call_log: [
          "waiting for getByRole('button', { name: 'Next' })",
          "the button is visible, disabled",
          "retrying click: the button is disabled",
        ],
      },
    }),
  },
};

/** A handler's own exception, which the message always names. */
export const ProgramException: Story = {
  args: {
    error: new ProgramError("handler_failed: TypeError: Cannot read properties of undefined", {
      code: "handler_failed",
      group: "program",
      retryable: false,
      details: {
        operation: "crawl_request.handle",
        url: "https://shop.test/product/42",
        exception_type: "TypeError",
        exception_message: "Cannot read properties of undefined (reading 'price')",
      },
    }),
  },
};

/** A refusal with no field but its code. */
export const Refused: Story = {
  args: {
    error: new PersonRequiredError("usage.raise: only a person gives this answer"),
  },
};

/** A code the catalog does not know: the engine's English message. */
export const UnknownCode: Story = {
  args: {
    error: new ProgramError("dataset_unknown: the crawl names no Dataset `prices`", {
      code: "dataset_unknown",
      group: "program",
      retryable: false,
    }),
  },
};
