// cases: cancel-mid-run
// A smoke script whose assertion fails on every runtime.
import assert from "node:assert/strict";

assert.equal("completed", "cancelled");
