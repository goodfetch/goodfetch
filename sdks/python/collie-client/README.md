# goodfetch-collie-client

The Python Collie client package, which both Products' SDKs share (ADR 0095). Area 11 fills it.

## Schemas

A program types data with a schema in its own types (ADR 0082): a Pydantic model class, any other type Pydantic's `TypeAdapter` accepts (`list[Book]`, a dataclass, a `TypedDict`), or a JSON Schema written by hand (`JSONSchema`), whose values are `JSONValue` and `JSONObject`. The engine receives the schema's validation side; a sent value is encoded by alias, a `Decimal` as an exact string and without computed fields, and must parse back as itself; an answer is decoded by the schema's own parse. The schema types the values a program sends and receives, a `Literal`, an `Annotated` or a union included. Results keep the fields this SDK does not know, and inputs refuse them. Only JSON crosses: a value that is not JSON (`NaN`, an object only pickle could carry, a key that is not a string) raises from the call that sent it, naming its path as a JSON Pointer.

Supported range, tested at its floor, which `uv.lock` pins:

| Library | Versions |
|---|---|
| Pydantic | 2.14.1 and later 2.x |
