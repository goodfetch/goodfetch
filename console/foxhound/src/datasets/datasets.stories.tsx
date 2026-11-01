// Each state of a crawl run's Datasets (TESTING.md, Components; spec #593,
// stories 204, 205, 207 and 233): the Datasets list with a schema and
// without, the Records grid holding the Unicode record of
// `goodfetch-test-support` with each cell isolated and kept out of
// translation, a grid with more Records to read, and the controls that ask
// the engine to filter, sort and keep columns, and the views a browser
// keeps offline (story 215). Foxhound's messages join the
// Console's before each story, as Foxhound's build entry adds them.
import type { DatasetRecord, DatasetSummary } from "@goodfetch/foxhound/generated";
import { i18n } from "@goodfetch/console/i18n";
import type { Meta, StoryObj } from "@storybook/react-vite";
// oxlint-disable-next-line import/no-relative-parent-imports -- the Unicode record is the one shared fixture file Rust and Python read too (ADR 0073), which no package exports to a browser.
import unicodeRecordText from "../../../../test-support/fixtures/unicode-record.json?raw";
import { foxhoundMessages } from "#messages.ts";
import { DatasetList, KeptViews, RecordsControls, RecordsGrid } from "./datasets.tsx";

/** Loads Foxhound's messages in the Console's locale. */
async function withFoxhoundMessages(): Promise<Record<string, never>> {
  const { locale } = i18n;
  if (locale === "en" || locale === "en-XA" || locale === "ar-XB") {
    i18n.load(locale, await foxhoundMessages(locale));
    i18n.activate(locale);
  }
  return {};
}

/** The Unicode record (ADR 0073) as one Record's value, a field per entry. */
function unicodeValue(): Record<string, string> {
  const parsed: unknown = JSON.parse(unicodeRecordText);
  const fields =
    typeof parsed === "object" && parsed !== null && "fields" in parsed ? parsed.fields : [];
  const value: Record<string, string> = {};
  if (Array.isArray(fields)) {
    for (const field of fields) {
      if (typeof field?.name === "string" && typeof field?.text === "string") {
        value[field.name] = field.text;
      }
    }
  }
  return value;
}

/** The Record numbered `number` holding `value`. */
function record(number: number, value: Record<string, unknown>): DatasetRecord {
  return {
    number,
    value,
    request_id: String(number),
    url: `https://shop.test/${String(number)}`,
    committed_at: "2026-10-24T09:00:00Z",
    provenance: { request_id: String(number) },
  };
}

const BOOKS: DatasetSummary = {
  name: "books",
  schema: {
    type: "object",
    required: ["title", "isbn"],
    properties: { title: { type: "string" }, isbn: { type: "string" } },
  },
  key: ["isbn"],
  committed: 1204,
  dropped: 3,
  rejected: 17,
};

const NOTES: DatasetSummary = { name: "notes", key: [], committed: 0, dropped: 0, rejected: 0 };

const meta: Meta<typeof RecordsGrid> = {
  title: "Datasets",
  component: RecordsGrid,
  loaders: [withFoxhoundMessages],
};

export default meta;

type Story = StoryObj<typeof RecordsGrid>;

/** The crawl's Datasets: one with a schema and a key, one with neither. */
export const Datasets: Story = {
  render: () => <DatasetList datasets={[BOOKS, NOTES]} />,
};

/** The Unicode record, each of its fields a cell isolated with `dir="auto"` and `translate="no"`. */
export const UnicodeRecord: Story = {
  args: {
    dataset: "unicode",
    columns: Object.keys(unicodeValue()),
    records: [record(1, unicodeValue())],
    more: false,
  },
};

/** A page of Records, more of them to read once the last come into view. */
export const MoreToRead: Story = {
  args: {
    dataset: "books",
    columns: ["title", "isbn"],
    records: [
      record(1, { title: "Dune", isbn: "0441013597" }),
      record(2, { title: "الخيميائي", isbn: "9789953680" }),
      record(3, { title: "Children of Dune" }),
    ],
    more: true,
  },
};

/** The views a browser keeps offline, shown while the engine is not reachable (story 215). */
export const KeptOffline: Story = {
  render: () => (
    <KeptViews
      views={[
        {
          run_id: "crawl-run",
          dataset: BOOKS,
          columns: ["title"],
          records: [
            { number: 1, value: { title: "Dune" } },
            { number: 2, value: { title: "الخيميائي" } },
          ],
        },
      ]}
    />
  ),
};

/** The controls that ask the engine to sort, filter and keep columns. */
export const Controls: Story = {
  render: () => (
    <RecordsControls
      fields={["title", "isbn"]}
      query={{ sort: { field: "title", direction: "descending" }, wall: "without" }}
      onApply={(): void => {}}
    />
  ),
};
