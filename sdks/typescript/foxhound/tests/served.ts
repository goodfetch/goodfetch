// The Connection's settings for a test crawling the Proving ground by its
// named hosts.
import type { FoxhoundSettings } from "@goodfetch/foxhound";
import type { ProvingGroundDescription } from "@goodfetch/test-support";

/**
 * The Connection's settings: plain HTTP fetches, and each of the Proving
 * ground's hosts resolved to its address and reachable although private.
 */
export function served(provingGround: ProvingGroundDescription): FoxhoundSettings {
  const hosts = Object.keys(provingGround.resolve);
  const resolve = Object.fromEntries(
    Object.entries(provingGround.resolve).map(([name, address]) => [name, [address]]),
  );
  return { network: { resolve }, egress: { privateRanges: hosts }, fetch: { tier: "http" } };
}
