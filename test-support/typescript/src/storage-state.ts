// A served test never writes a cookie or storage state the Console did not
// set (ADR 0068): it signs in through `signIn`, so a test that gives its
// context Playwright's `storageState` fails.
import { TestSupportError } from "./errors.ts";

/** The options of a Playwright test that could give its context a storage state. */
export interface StorageStateOptions {
  readonly storageState?: unknown;
  readonly contextOptions?: { readonly storageState?: unknown } | undefined;
}

/** Fails with `storage_state_refused` when `options` give a storage state, directly or through `contextOptions`. */
export function refuseStorageState(options: StorageStateOptions): void {
  if (options.storageState !== undefined || options.contextOptions?.storageState !== undefined) {
    throw new TestSupportError(
      "storage_state_refused",
      "storageState is set: a served test signs in through the signIn fixture, never with cookies or storage the Console did not set (ADR 0068)",
    );
  }
}
