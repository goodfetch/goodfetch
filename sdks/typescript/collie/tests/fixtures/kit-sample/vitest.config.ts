// The sample's own Vitest configuration: Vitest's defaults, the sample
// alone, no setup file and no plugin of Goodfetch's.
import { defineConfig, type ViteUserConfig } from "vitest/config";

const config: ViteUserConfig = defineConfig({ test: { include: ["sample.vitest.ts"] } });

export default config;
