// Complete error details (ADRs 0035, 0079, 0092, 0101, 0106, 0114, 0115 and
// 0118), on the Scripted engine as a Collie engine: each failure carries
// every detail its code declares. A fill whose one-time code never came
// raises `SecretUnavailableError` with `failure: "no_code"`, and
// `page_navigated_during_call` raises `PageNavigatedDuringCallError`,
// retryable, naming the document the call read and the one that replaced it.
// A navigation a site rate-limits raises `BlockedError` with the Block
// cause's wire value, `rate_limit`, and `retryAfter` the point in time the
// site asked for, in UTC; a cause no catalog names is kept as received, with
// no time. A Model provider rate-limiting raises `ModelUnavailableError`
// with `retryAfter` as a duration in milliseconds; `effect_uncertain`
// carries `UncertainEffect` with `request: null` on a Collie Run;
// `artifact_unavailable_offline` carries `pinnedBy` as a typed list of
// product and plugin pinners; `too_many_redirects` carries `hops` and
// `limit`. Played attached, and as a Daemon, whose script exits 0 only once
// every step played. Python's `test_complete_error_details.py` and Rust's
// `complete_error_details.rs` are its twins.
import {
  ArtifactUnavailableOfflineError,
  BlockedError,
  Collie,
  EffectUncertainError,
  type GoodfetchError,
  ModelUnavailableError,
  PageError,
  PageNavigatedDuringCallError,
  RefusedError,
  SecretUnavailableError,
  TooManyRedirectsError,
  UnavailableError,
} from "@goodfetch/collie";
import {
  type ClientCaseRecord,
  goodfetchFailure,
  scriptedEngineEnv,
  startScriptedDaemon,
  test,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** What the script's calls answered. */
interface Played {
  /** The fill of the one-time code no provider made. */
  readonly noCode: GoodfetchError;
  /** The click whose document a navigation it did not start replaced. */
  readonly navigated: GoodfetchError;
  /** The navigation the site rate-limited. */
  readonly rateLimited: GoodfetchError;
  /** The navigation the site refused with a cause no catalog names. */
  readonly unknownCause: GoodfetchError;
  /** The call whose Model provider rate-limited it with a wait duration. */
  readonly modelUnavailable: GoodfetchError;
  /** The call whose Effect was left uncertain on a Collie Run. */
  readonly effectUncertain: GoodfetchError;
  /** The call refused by `offline` for a pinned browser artifact. */
  readonly artifactOffline: GoodfetchError;
  /** The navigation that exceeded the browser's redirect limit. */
  readonly tooManyRedirects: GoodfetchError;
}

/** Plays the script's calls on `collie`, then closes it. */
async function played(collie: Collie): Promise<Played> {
  const run = await collie.start();
  const page = await run.newPage();
  const noCode = await goodfetchFailure(page.locator("#otp").fill("%shop_totp%"));
  const navigated = await goodfetchFailure(page.locator("#book").click());
  const rateLimited = await goodfetchFailure(page.goto("https://shop.test/deals"));
  const unknownCause = await goodfetchFailure(page.goto("https://shop.test/abroad"));
  const modelUnavailable = await goodfetchFailure(page.locator("#summarize").click());
  const effectUncertain = await goodfetchFailure(page.locator("#pay").click());
  const artifactOffline = await goodfetchFailure(page.goto("https://shop.test/offline"));
  const tooManyRedirects = await goodfetchFailure(page.goto("https://shop.test/bounce"));
  await run.finish();
  await collie.close();
  return {
    noCode,
    navigated,
    rateLimited,
    unknownCause,
    modelUnavailable,
    effectUncertain,
    artifactOffline,
    tooManyRedirects,
  };
}

/** The case's checks, the same in both modes. */
async function checkPlayed(clientCase: ClientCaseRecord, answered: Played): Promise<void> {
  const {
    noCode,
    navigated,
    rateLimited,
    unknownCause,
    modelUnavailable,
    effectUncertain,
    artifactOffline,
    tooManyRedirects,
  } = answered;
  await clientCase.check(
    "a one-time code that never came raises SecretUnavailableError saying no_code",
    () => {
      expect(noCode).toBeInstanceOf(SecretUnavailableError);
      expect(noCode).toBeInstanceOf(UnavailableError);
      expect(noCode).toMatchObject({
        code: "secret_unavailable",
        group: "unavailable",
        retryable: false,
        fix: "Check the TOTP seed of the vault's shop item",
      });
      expect(noCode.details).toEqual({
        placeholder: "shop_totp",
        reference: "bw:shop/totp",
        provider: "bw",
        failure: "no_code",
      });
    },
  );
  await clientCase.check(
    "a navigation the call did not start raises PageNavigatedDuringCallError naming both documents",
    () => {
      expect(navigated).toBeInstanceOf(PageNavigatedDuringCallError);
      expect(navigated).toBeInstanceOf(PageError);
      expect(navigated).toMatchObject({
        code: "page_navigated_during_call",
        group: "page",
        retryable: true,
        url: "https://travel.test/books",
        fix: "Await the Collie call before your driver acts on the page.",
      });
      expect(navigated.details).toEqual({ navigated_to: "https://travel.test/books/1" });
    },
  );
  await clientCase.check(
    "a rate-limited navigation raises BlockedError with rate_limit and retryAfter a point in time",
    () => {
      expect(rateLimited).toBeInstanceOf(BlockedError);
      expect(rateLimited).toMatchObject({
        code: "blocked",
        group: "page",
        retryable: false,
        url: "https://shop.test/deals",
        retryAfter: new Date("2026-11-21T08:00:00Z"),
      });
      expect(rateLimited.details).toEqual({
        cause: "rate_limit",
        status: 429,
        retry_after: "2026-11-21T10:00:00+02:00",
      });
    },
  );
  await clientCase.check(
    "a Block cause no catalog names is kept as received, with no retryAfter",
    () => {
      expect(unknownCause).toBeInstanceOf(BlockedError);
      expect(unknownCause).toMatchObject({ url: "https://shop.test/abroad", retryAfter: null });
      expect(unknownCause.details).toEqual({ cause: "geo_block", status: 451, retry_after: null });
    },
  );
  await clientCase.check(
    "a rate-limited Model provider raises ModelUnavailableError with retryAfter a duration in milliseconds",
    () => {
      expect(modelUnavailable).toBeInstanceOf(ModelUnavailableError);
      expect(modelUnavailable).toBeInstanceOf(UnavailableError);
      expect(modelUnavailable).toMatchObject({
        code: "model_unavailable",
        group: "unavailable",
        retryable: true,
        retryAfter: 1500,
        typedDetails: {
          reason: "rate_limited",
          retryAfter: 1500,
        },
      });
    },
  );
  await clientCase.check(
    "an uncertain Effect on a Collie Run raises EffectUncertainError with request null",
    () => {
      expect(effectUncertain).toBeInstanceOf(EffectUncertainError);
      expect(effectUncertain).toBeInstanceOf(PageError);
      expect(effectUncertain).toMatchObject({
        code: "effect_uncertain",
        group: "page",
        retryable: false,
        typedDetails: {
          entry: "effect-1",
          key: "charge-order-42",
          request: null,
          reason: "browser closed before confirmation",
          firstCommand: "locator.click",
          since: new Date("2026-11-21T08:00:00Z"),
        },
      });
    },
  );
  await clientCase.check(
    "an offline artifact refusal raises ArtifactUnavailableOfflineError with pinnedBy as a typed list",
    () => {
      expect(artifactOffline).toBeInstanceOf(ArtifactUnavailableOfflineError);
      expect(artifactOffline).toBeInstanceOf(RefusedError);
      expect(artifactOffline).toMatchObject({
        code: "artifact_unavailable_offline",
        group: "refused",
        retryable: false,
        fix: "Run `collie browser install` where the cache is writable.",
        typedDetails: {
          artifact: {
            kind: "browser",
            name: "chrome-for-testing",
            build: "141.0.7390.54-linux64",
          },
          pinnedBy: [
            { kind: "product", name: "collie", version: "0.0.0" },
            { kind: "plugin", name: "demo-plugin", version: "1.2.0" },
          ],
          cache: "/var/cache/goodfetch/browsers",
        },
      });
    },
  );
  await clientCase.check(
    "a redirect loop raises TooManyRedirectsError with hops and the browser limit",
    () => {
      expect(tooManyRedirects).toBeInstanceOf(TooManyRedirectsError);
      expect(tooManyRedirects).toBeInstanceOf(PageError);
      expect(tooManyRedirects).toMatchObject({
        code: "too_many_redirects",
        group: "page",
        retryable: false,
        url: "https://shop.test/bounce",
        typedDetails: {
          hops: ["https://shop.test/bounce/1", "https://shop.test/bounce/2"],
          limit: { kind: "browser", count: 20 },
        },
      });
    },
  );
}

const COVERS = [
  "story:11-38",
  "story:11-39",
  "story:11-191",
  "error:secret_unavailable",
  "error:page_navigated_during_call",
  "error:blocked",
  "error:model_unavailable",
  "error:effect_uncertain",
  "error:artifact_unavailable_offline",
  "error:too_many_redirects",
  "block-cause:rate_limit",
] as const;

describe("A Collie Connection on the Scripted engine", () => {
  test(
    "reads every detail each failure's code declares",
    {
      tags: ["client_case/complete-error-details", "attached", ...COVERS],
    },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/complete-error-details.json");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const answered = await played(await Collie.connect({ env }));

      await checkPlayed(clientCase, answered);
    },
  );

  test(
    "reads every detail each failure's code declares from a Daemon",
    {
      tags: ["client_case/complete-error-details", "daemon", ...COVERS],
    },
    async ({ testEnv, processes, clientCase }) => {
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "run/complete-error-details-from-a-daemon.json",
        "socket",
      );
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const answered = await played(await Collie.connect({ env: testEnv.env, engine: "daemon" }));
      const exit = await daemon.exit;

      await checkPlayed(clientCase, answered);
      await clientCase.check(
        "the Daemon played its script through, so the Client sent exactly what it holds",
        () => expect(exit).toMatchObject({ code: 0 }),
      );
    },
  );
});
