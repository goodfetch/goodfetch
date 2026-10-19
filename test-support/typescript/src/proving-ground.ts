// The Proving ground `goodfetch-test-env --proving-ground <seed>` starts for
// one test (ADRs 0059 and 0063): its address, version and seed, the
// `resolve` mapping of every host name it serves to loopback, the file of its
// certificate authority, and its network fixtures, so a test reaches its
// pages, its forward proxies and its connection record with no setup of its
// own. TypeScript reads what the Rust setup started, and starts nothing.
import { TestSupportError } from "./errors.ts";
import type { TestEnvDescription } from "./test-env.ts";

/** A forward proxy's Basic credentials, derived from the seed. */
export interface ProxyCredentials {
  readonly username: string;
  readonly password: string;
}

/** The network fixtures `serve` starts beside its pages. */
export interface ProvingGroundFixtures {
  /** Each forward proxy's address, by name: `open`, `refusing_tunnels`, `down` and one per proxy kind. */
  readonly proxies: Readonly<Record<string, string>>;
  /** The credentials the authenticated proxies accept. */
  readonly proxy_credentials: ProxyCredentials;
  /** The DNS fixture's UDP address. */
  readonly dns: string;
  /** The watched target's address, which answers `200 reached`. */
  readonly target: string;
  /** The URL of the connection record: every connection accepted and every relay a proxy made. */
  readonly connections: string;
}

/** A started Proving ground, as `goodfetch-test-env` describes it. */
export interface ProvingGroundDescription {
  /** The address it answers on, such as `127.0.0.1:51234`. */
  readonly address: string;
  /** Its version, `<workspace version>+<commit>`. */
  readonly version: string;
  /** The seed every page and expected result comes from. */
  readonly seed: number;
  /** Every host name it serves, mapped to the IP of its address, as the `resolve` setting takes them. */
  readonly resolve: Readonly<Record<string, string>>;
  /** The file of its certificate authority, as PEM, in the test's folder. */
  readonly ca: string;
  /** The addresses of its network fixtures. */
  readonly fixtures: ProvingGroundFixtures;
}

/** The Proving ground a description names; `proving_ground_unseeded` when the test asked for none. */
export function provingGroundOf(description: TestEnvDescription): ProvingGroundDescription {
  if (description.proving_ground === undefined) {
    throw new TestSupportError(
      "proving_ground_unseeded",
      'the test takes `provingGround` but sets no seed: add `test.override("provingGroundSeed", <seed>)`',
    );
  }
  return description.proving_ground;
}
