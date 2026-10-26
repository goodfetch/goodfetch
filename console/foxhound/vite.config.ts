// Foxhound's Console build (ADRs 0057, 0068 and 0087): `xtask console
// build` runs it into a folder of its own, then compresses every file it
// writes with brotli, which Foxhound's executable embeds and serves as they
// are. It compiles Collie's screens, through `@goodfetch/console`'s build
// entry, with Foxhound's, and both packages' messages through this
// package's Lingui configuration; its public files are Collie's.
//
// The production Content Security Policy allows the Console's own files
// only (ADR 0031), so the build inlines nothing: no asset as a `data:`
// address, and no module preload polyfill, which Vite would add as code of
// its own. Everything it reads is installed: the build calls nowhere (ADR
// 0043).
import { lingui } from "@lingui/vite-plugin";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig, type UserConfig } from "vite";

const config: UserConfig = defineConfig({
  plugins: [
    react(),
    lingui({ macroTransform: true, configPath: "./lingui.config.ts" }),
    tailwindcss(),
  ],
  publicDir: "../collie/public",
  build: {
    assetsInlineLimit: 0,
    modulePreload: { polyfill: false },
    emptyOutDir: true,
    reportCompressedSize: false,
  },
});

export default config;
