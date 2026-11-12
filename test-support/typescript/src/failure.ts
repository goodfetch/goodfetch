// The error a call fails with, for a test that asserts on it: a call that
// succeeds fails the test, where a bare `catch` would pass it.

/** The error `promise` rejects with. */
export async function failure(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise;
  } catch (error) {
    return error;
  }
  throw new Error("the call succeeded");
}
