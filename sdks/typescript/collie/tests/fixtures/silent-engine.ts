#!/usr/bin/env node
// An engine stand-in that answers nothing: it writes its process ID to the
// file `SILENT_ENGINE_PID_FILE` names, reads its input to the end, and exits
// with code 0 once that input ends, as an Attached engine does.
import { writeFileSync } from "node:fs";

const file = process.env["SILENT_ENGINE_PID_FILE"];
if (file === undefined) {
  throw new TypeError("SILENT_ENGINE_PID_FILE names no file");
}
writeFileSync(file, String(process.pid));
process.stdin.resume();
