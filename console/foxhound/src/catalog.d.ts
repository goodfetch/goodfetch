// A message catalog, which the Lingui Vite plugin compiles when imported
// with `?lingui`.
declare module "*?lingui" {
  import type { Messages } from "@lingui/core";

  export const messages: Messages;
}
