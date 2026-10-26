// Each state of the SQL tab (TESTING.md, Components; spec #593, stories 210
// and 211): before any statement, while one runs, a result with mixed
// directions in its cells, a result past `dataset.query.max_rows` with
// rows counted, not returned, and a statement refused with `query_invalid`.
import { QueryInvalidError } from "@goodfetch/foxhound/generated";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { SqlTabView } from "./sql-tab.tsx";

const RUN_ID = "shop-monday";

const meta: Meta<typeof SqlTabView> = {
  title: "Foxhound/SQL tab",
  component: SqlTabView,
  args: {
    runId: RUN_ID,
    ran: { kind: "idle" },
    page: 0,
    onPage: () => {},
    onRun: () => {},
  },
};

export default meta;

type Story = StoryObj<typeof SqlTabView>;

/** Before any statement ran. */
export const Idle: Story = {};

/** While the engine runs the statement. */
export const Running: Story = { args: { ran: { kind: "running" } } };

/** A result whose cells run left to right and right to left. */
export const Answered: Story = {
  args: {
    ran: {
      kind: "answered",
      result: {
        columns: [
          { name: "title", type: "VARCHAR" },
          { name: "runs", type: "BIGINT" },
        ],
        rows: [
          ["Café 1", 2],
          ["كتاب ٢", 1],
          ["שלום 3", 1],
        ],
        total: 3,
      },
    },
  },
};

/** A result past `dataset.query.max_rows`: the rest counted, not returned. */
export const Counted: Story = {
  args: {
    ran: {
      kind: "answered",
      result: {
        columns: [{ name: "number", type: "BIGINT" }],
        rows: Array.from({ length: 250 }, (_, index) => [index + 1]),
        total: 1_000_000,
      },
    },
    page: 1,
  },
};

/** A write the SQL tab refused. */
export const Refused: Story = {
  args: {
    ran: {
      kind: "refused",
      error: new QueryInvalidError(
        "The SQL tab did not run the statement: Only SELECT statements can be serialized to json!",
        { details: { reason: "not_a_read" } },
      ),
    },
  },
};
