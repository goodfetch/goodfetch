// Foxhound's Console build entry (ADR 0087): Collie's build entry, started
// with Foxhound's build and Foxhound's messages added to the Console's.
import { startConsole } from "@goodfetch/console/start";
import { FOXHOUND_BUILD } from "./foxhound-build.ts";
import { foxhoundMessages } from "./messages.ts";

await startConsole(FOXHOUND_BUILD, { messages: foxhoundMessages });
