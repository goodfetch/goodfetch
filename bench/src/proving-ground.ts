// The runner's client of a running Proving ground: its task list and its
// oracle, on the address it answers on. Every figure a run records comes from
// tasks this Proving ground serves and judges.
import { BenchError } from "./errors.ts";
import { parserOf } from "./schema.ts";

/** One task of the list, as `GET /tasks` serves it. */
export interface ListedTask {
  readonly id: string;
  readonly family: string;
  readonly kind: string;
  readonly start_url: string;
  readonly description: string;
  /** Changes whenever the task or its expected result changes (ADR 0063). */
  readonly digest: string;
}

/** What `GET /tasks` serves. */
export interface TaskList {
  readonly proving_ground: { readonly version: string; readonly seed: number };
  readonly tasks: readonly ListedTask[];
}

/** What a tool said of its own result. */
export type ToolStatus = "success" | "needs-review" | "failure";

/** A failing tool's error: its Error code and what the code names. */
export interface SubmittedError {
  readonly code: string;
  readonly network_error?: string;
  readonly rule?: string;
  readonly setting?: string;
}

/** A tool's result for one task, as `POST /oracle` takes it. */
export interface Submission {
  readonly task: string;
  readonly status: ToolStatus;
  readonly records: readonly unknown[];
  readonly error?: SubmittedError;
}

/** The oracle's judgement of a result. */
export type Outcome = "verified" | "incomplete" | "false_success" | "failed";

/** The oracle's answer: its outcome, what was missing, extra or wrong, and its own time. */
export interface Judgement {
  readonly task: string;
  readonly outcome: Outcome;
  readonly missing: readonly string[];
  readonly extra: readonly unknown[];
  readonly wrong: readonly unknown[];
  readonly oracle_time_microseconds: number;
}

const taskList = parserOf<TaskList>({
  type: "object",
  required: ["proving_ground", "tasks"],
  properties: {
    proving_ground: {
      type: "object",
      required: ["version", "seed"],
      properties: { version: { type: "string" }, seed: { type: "integer", minimum: 0 } },
    },
    tasks: {
      type: "array",
      items: {
        type: "object",
        required: ["id", "family", "kind", "start_url", "description", "digest"],
        properties: {
          id: { type: "string" },
          family: { type: "string" },
          kind: { type: "string" },
          start_url: { type: "string" },
          description: { type: "string" },
          digest: { type: "string" },
        },
      },
    },
  },
});

const judgement = parserOf<Judgement>({
  type: "object",
  required: ["task", "outcome", "missing", "extra", "wrong", "oracle_time_microseconds"],
  properties: {
    task: { type: "string" },
    outcome: { enum: ["verified", "incomplete", "false_success", "failed"] },
    missing: { type: "array", items: { type: "string" } },
    extra: { type: "array" },
    wrong: { type: "array" },
    oracle_time_microseconds: { type: "integer", minimum: 0 },
  },
});

/** A running Proving ground. */
export interface ProvingGround {
  /** The address it answers on, such as `127.0.0.1:51234`. */
  readonly address: string;
  /** Its task list. */
  tasks(): Promise<TaskList>;
  /** The oracle's judgement of `submission`. */
  judge(submission: Submission): Promise<Judgement>;
}

async function exchange(url: string, init: RequestInit): Promise<unknown> {
  let response: Response;
  try {
    response = await fetch(url, init);
  } catch (error) {
    throw new BenchError("oracle_refused", `the Proving ground at ${url} is unreachable`, {
      cause: error,
    });
  }
  const text = await response.text();
  if (!response.ok) {
    throw new BenchError("oracle_refused", `${url} answered ${response.status}: ${text}`);
  }
  return JSON.parse(text);
}

/** The client of the Proving ground that answers on `address`. */
export function provingGround(address: string): ProvingGround {
  return {
    address,
    async tasks() {
      return taskList.parse(await exchange(`http://${address}/tasks`, {}), "task list");
    },
    async judge(submission) {
      const answer = await exchange(`http://${address}/oracle`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(submission),
      });
      return judgement.parse(answer, "oracle answer");
    },
  };
}
