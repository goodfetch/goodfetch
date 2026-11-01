// Collie's Console build entry (ADR 0087): Collie's build, started as
// `start.tsx` starts every Product's.
import { COLLIE_BUILD } from "./console-build.ts";
import { startConsole } from "./start.tsx";

await startConsole(COLLIE_BUILD);
