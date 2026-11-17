// The values a page's fields answer, typed from their declaration (ADR
// 0147): a generated member whose result the catalog marks `typed_by` one
// of its parameters answers each field it declares under the field's name,
// with the value the declaration gives. Hand-written, since no type of the
// bundle can say that a result's names and types follow a parameter's; the
// types read the declaration's shape alone, so they import nothing.

/** The value a value field's Value transform gives: a `number` reads a number, every other a text. */
type TransformValue<Kind> = Kind extends "number" ? number : string;

/** The `kind` of a field's transform, `text` when it declares none. */
type TransformKind<Transform> = Transform extends { readonly kind: infer Kind } ? Kind : "text";

/** A value field's value: its transform's, or null where the page did not give it. */
type ValueFieldValue<Field> =
  | (Field extends { readonly transform?: infer Transform }
      ? TransformValue<TransformKind<Transform>>
      : string)
  | null;

/**
 * The value one field declaration answers (ADR 0147): a list's is an array
 * of its value field's values, rows' an array of objects of their fields'
 * values, and a value field's its own; each value is null where the page
 * did not give it, and the result's `unread` says why.
 */
export type FieldValue<Field> = Field extends { readonly list: infer Item }
  ? ValueFieldValue<Item>[]
  : Field extends { readonly rows: unknown; readonly fields: infer Fields }
    ? { -readonly [Name in keyof Fields]: ValueFieldValue<Fields[Name]> }[]
    : ValueFieldValue<Field>;

/** Each field's value under its name, as `Fields` declares them. */
export type FieldValues<Fields> = { -readonly [Name in keyof Fields]: FieldValue<Fields[Name]> };

/** `Result`, whose `Property` holds the values of the fields `Fields` declares. */
export type TypedBy<Result, Property extends keyof Result, Fields> = {
  [Key in keyof Result]: Key extends Property ? FieldValues<Fields> : Result[Key];
};

/**
 * `answer`, its `Property` typed by the fields `Fields` declares: the
 * engine answers each declared field under its name with the value its
 * declaration gives (ADR 0147), and engine answers are not validated at
 * runtime beyond the protocol version check.
 */
export function typedBy<Result, Property extends keyof Result, Fields>(
  answer: Result,
): TypedBy<Result, Property, Fields> {
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the engine answers each declared field under its name with the value its declaration gives (ADR 0147), which is what `TypedBy` says.
  return answer as TypedBy<Result, Property, Fields>;
}
