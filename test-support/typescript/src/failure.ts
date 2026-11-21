// The error a call fails with, for a test that asserts on it: a call that
// succeeds fails the test, where a bare `catch` would pass it.
import { type GoodfetchError, isGoodfetchError } from "@goodfetch/collie-client/generated";

/** The error `promise` rejects with. */
export async function failure(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise;
  } catch (error) {
    return error;
  }
  throw new Error("the call succeeded");
}

/** The Goodfetch error `promise` rejects with; any other failure fails the test. */
export async function goodfetchFailure(promise: Promise<unknown>): Promise<GoodfetchError> {
  const error = await failure(promise);
  if (!isGoodfetchError(error)) {
    throw new Error(`the call failed with ${String(error)}, no Goodfetch error`);
  }
  return error;
}
