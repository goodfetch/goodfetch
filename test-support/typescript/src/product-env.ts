// The environment map an SDK takes in a test (ADRs 0069, 0090 and 0094):
// the test's cleared environment, `GOODFETCH_HOME` among it, with the
// engine-path variable of the SDK's Product pointing at that Product's
// executable from the test tool manifest.
import { toolPath, type TestEnvDescription } from "./test-env.ts";

/** A Product; a test runs Collie's executable unless it needs Foxhound (ADR 0094). */
export type Product = "collie" | "foxhound";

const ENGINE_PATH_VARIABLES = {
  collie: "GOODFETCH_COLLIE_ENGINE_PATH",
  foxhound: "GOODFETCH_FOXHOUND_ENGINE_PATH",
} as const satisfies Record<Product, string>;

/** The environment map an SDK of `product` takes; `tool_missing` when the manifest lists no executable of it. */
export function productEnv(
  description: TestEnvDescription,
  product: Product,
): Readonly<Record<string, string>> {
  return {
    ...description.env,
    [ENGINE_PATH_VARIABLES[product]]: toolPath(description.tools, product),
  };
}
