// What the Console needs from its target for a Run's files (ADRs 0031,
// 0067 and 0068): a large file arrives by its handle through the Client's
// body fetcher, to a sign-in only, never through the protocol's messages;
// and a file to show is shown only from the content origin, by an address
// signed for it, sandboxed and with no cookie of the Console's.
import { expect, test } from "@goodfetch/test-support/conformance";
import { httpBodyFetcher } from "@goodfetch/console/client/browser";
import {
  cookieHeader,
  NOT_ON_THE_STUB_GATEWAY,
  skipBehindGateway,
  startRun,
  storeBody,
} from "./target.ts";

/** A file larger than any protocol message may be: 20 MiB. */
const LARGE = 20 * 1024 * 1024;

test(
  "a-large-file-arrives-by-handle-to-a-sign-in-only",
  {
    annotation: [
      { type: "capability", description: "bodies" },
      { type: "covers", description: "story:12-127" },
    ],
  },
  async ({ page, context, daemon, signIn }) => {
    skipBehindGateway(daemon, NOT_ON_THE_STUB_GATEWAY.bodies);
    await signIn.withLink(page);
    await startRun(daemon, "first");
    const bytes = new Uint8Array(LARGE).map((_, index) => index % 251);
    const handle = await storeBody(daemon, "first", bytes);
    const origin = new URL(daemon.url).origin;
    const cookie = await cookieHeader(context, daemon.url);
    const signedIn: typeof fetch = async (input, init) =>
      fetch(input, { ...init, headers: { Cookie: cookie } });

    const fetched = await httpBodyFetcher(origin, signedIn)(handle);

    expect(fetched.length).toBe(LARGE);
    expect(Buffer.from(fetched).equals(Buffer.from(bytes))).toBe(true);
    await expect(httpBodyFetcher(origin, fetch)(handle)).rejects.toThrow(/answered 401/);
  },
);

test(
  "a-file-is-shown-only-from-the-content-origin-sandboxed",
  {
    annotation: [
      { type: "capability", description: "content-origin" },
      { type: "covers", description: "story:12-127" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    skipBehindGateway(daemon, NOT_ON_THE_STUB_GATEWAY.bodies);
    await signIn.withLink(page);
    await startRun(daemon, "first");
    const html = "<!doctype html><title>A body</title><main>A body.</main><script>1</script>";
    const handle = await storeBody(daemon, "first", new TextEncoder().encode(html));

    const address = await page.evaluate(async (path) => {
      const response = await fetch(`/content-address/${path}?type=html`);
      const answer: unknown = await response.json();
      return typeof answer === "object" && answer !== null && "url" in answer
        ? String(answer.url)
        : "";
    }, handle);

    expect(new URL(address).origin).not.toBe(new URL(daemon.url).origin);
    const shown = await fetch(address);
    expect(shown.status).toBe(200);
    expect(await shown.text()).toBe(html);
    const policy = shown.headers.get("content-security-policy") ?? "";
    expect(policy).toMatch(/(^|;\s*)sandbox(;|$|\s)/);
    expect(policy).not.toContain("allow-scripts");
    expect(shown.headers.get("set-cookie")).toBeNull();
    // An address not signed for that file is refused.
    const forged = new URL(address);
    forged.pathname = forged.pathname.replace(/.$/, (last) => (last === "0" ? "1" : "0"));
    expect((await fetch(forged)).status).toBeGreaterThanOrEqual(400);
  },
);
