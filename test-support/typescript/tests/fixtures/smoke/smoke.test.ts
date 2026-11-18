// The smoke scripts beside this file, as an SDK's tests run its own.
import { join } from "node:path";
import { smokeTests } from "@goodfetch/test-support/smoke";

smokeTests({ folder: join(import.meta.dirname, "scripts") });
