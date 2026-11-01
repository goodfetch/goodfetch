// What the committed protocol contract allows on the wire: the JSON Schema
// bundle and the catalog `xtask codegen` writes into
// `crates/goodfetch-protocol/generated/` (ADR 0038), compiled with Ajv, the one
// runtime validator of the TypeScript test tools (spec #326, ambiguity 6).
// The files are imported as JSON modules, so this module also runs in Vitest
// Browser Mode, where no file system is.
import { Ajv2020, type ValidateFunction } from "ajv/dist/2020.js";
import { fullFormats } from "ajv-formats/dist/formats.js";
import bundle from "../../../crates/goodfetch-protocol/generated/bundle.json" with { type: "json" };
import catalog from "../../../crates/goodfetch-protocol/generated/catalog.json" with { type: "json" };
import type { CancelParams, CreditParams, Product } from "@goodfetch/collie-client/generated";

/** Where the bundle's `$defs` are registered, so each catalog `$ref` resolves against them. */
const BUNDLE_ID = "https://goodfetch.dev/schemas/protocol-bundle.json";

const ajv = new Ajv2020({ allErrors: true });
// schemars marks each field with the protocol version it arrived in.
ajv.addKeyword("arrived_in");
ajv.addFormat("date-time", fullFormats["date-time"]);
for (const [format, minimum, maximum] of [
  ["uint8", 0, 2 ** 8 - 1],
  ["uint16", 0, 2 ** 16 - 1],
  ["int32", -(2 ** 31), 2 ** 31 - 1],
  ["uint32", 0, 2 ** 32 - 1],
  ["int64", Number.MIN_SAFE_INTEGER, Number.MAX_SAFE_INTEGER],
  ["uint64", 0, Number.MAX_SAFE_INTEGER],
] as const) {
  ajv.addFormat(format, {
    type: "number",
    validate: (value: number) =>
      Number.isSafeInteger(value) && value >= minimum && value <= maximum,
  });
}
ajv.addSchema({ $id: BUNDLE_ID, $defs: bundle.$defs });

/** A schema of the bundle or the catalog: a `$ref` into the bundle's `$defs`. */
interface Reference {
  readonly $ref: string;
}

/** What one schema of the contract finds wrong with a value: nothing, or Ajv's text. */
export type Problems = string | undefined;

/** A value checked against a schema: of its type, or broken, with Ajv's text. */
export type Checked<T> =
  | { readonly holds: true; readonly value: T }
  | { readonly holds: false; readonly problems: string };

/** Checks values against one schema of the contract, whose values are `T`s. */
export class Schema<T = unknown> {
  readonly #validate: ValidateFunction<T>;

  constructor(reference: Reference) {
    const validate = ajv.getSchema<T>(`${BUNDLE_ID}${reference.$ref}`);
    if (validate === undefined) {
      throw new Error(`invariant: the bundle defines ${reference.$ref}`);
    }
    this.#validate = validate;
  }

  /** `value` as a `T`, or what breaks this schema in it. */
  check(value: unknown): Checked<T> {
    return this.#validate(value)
      ? { holds: true, value }
      : { holds: false, problems: ajv.errorsText(this.#validate.errors) };
  }

  /** What breaks this schema in `value`, or `undefined` when it holds. */
  problems(value: unknown): Problems {
    const checked = this.check(value);
    return checked.holds ? undefined : checked.problems;
  }
}

/** A request's id, which its reply carries back. */
export type RequestId = number | string;

/** A JSON-RPC request, as the bundle's `Request` takes it. */
export interface WireRequest {
  readonly jsonrpc: "2.0";
  readonly id: RequestId;
  readonly method: string;
  readonly params?: unknown;
}

/** A JSON-RPC notification, as the bundle's `Notification` takes it. */
export interface WireNotification {
  readonly jsonrpc: "2.0";
  readonly method: string;
  readonly params?: unknown;
}

/** What `$/credit` grants: more events, or chunks, for a streaming call; the bundle's generated type, so its schema and this type cannot part. */
export type WireCredit = Readonly<CreditParams>;

/** What `$/cancel` names: the call to stop; the bundle's generated type. */
export type WireCancel = Readonly<CancelParams>;

/** An operation of the catalog: what its params and its result must be. */
export interface OperationContract {
  readonly params: Schema;
  readonly result: Schema;
}

/** An Error code of the catalog: its group's numeric class, and whether a retry may succeed. */
export interface ErrorCodeContract {
  readonly jsonRpcClass: number;
  readonly retryable: boolean;
}

/** One Product's part of the contract: what its engine takes and sends. */
export interface ProductContract {
  readonly product: Product;
  readonly protocolVersion: string;
  /** The JSON-RPC envelopes and the connection's own messages. */
  readonly request: Schema<WireRequest>;
  readonly notification: Schema<WireNotification>;
  readonly helloParams: Schema;
  readonly cancelParams: Schema<WireCancel>;
  readonly creditParams: Schema<WireCredit>;
  readonly operations: ReadonlyMap<string, OperationContract>;
  /** Each event's payload schema, by its catalog name. */
  readonly events: ReadonlyMap<string, Schema>;
  readonly errorCodes: ReadonlyMap<string, ErrorCodeContract>;
}

/** Whether an entry marked with `entry` belongs to `product`'s catalog: Foxhound's contains Collie's (ADR 0095). */
function carries(product: Product, entry: string): boolean {
  return entry === "collie" || product === "foxhound";
}

/** The committed contract of `product`'s engine. */
export function productContract(product: Product): ProductContract {
  const groups = new Map(catalog.error_groups.map((group) => [group.name, group.json_rpc_class]));
  return {
    product,
    protocolVersion: catalog.protocol_version,
    request: new Schema({ $ref: "#/$defs/Request" }),
    notification: new Schema({ $ref: "#/$defs/Notification" }),
    helloParams: new Schema(bundle.connection.hello.params),
    cancelParams: new Schema(bundle.connection.cancel.params),
    creditParams: new Schema(bundle.connection.credit.params),
    operations: new Map(
      catalog.operations
        .filter((operation) => carries(product, operation.product))
        .map((operation) => [
          operation.name,
          {
            params: new Schema(operation.params),
            result: new Schema(operation.result),
          },
        ]),
    ),
    events: new Map(
      catalog.events
        .filter((event) => carries(product, event.product))
        .map((event) => [event.name, new Schema(event.payload)]),
    ),
    errorCodes: new Map(
      catalog.error_codes
        .filter((code) => carries(product, code.product))
        .map((code) => {
          const jsonRpcClass = groups.get(code.group);
          if (jsonRpcClass === undefined) {
            throw new Error(`invariant: the catalog defines the group of ${code.name}`);
          }
          return [code.name, { jsonRpcClass, retryable: code.retryable }];
        }),
    ),
  };
}
