// The Proving ground `goodfetch-test-env --proving-ground <seed>` starts for
// one test (ADRs 0059 and 0063): its address, version and seed, the
// `resolve` mapping of every host name it serves to loopback, the file of its
// certificate authority, and its network fixtures, so a test reaches its
// pages, its forward proxies and its connection record with no setup of its
// own. TypeScript reads what the Rust setup started, and starts nothing.
// Its routes come from `test-support/fixtures/proving-ground/routes.json`,
// which `xtask codegen` writes from the Proving ground's own interface.
import type { ValidateFunction } from "ajv/dist/2020.js";
import { TestSupportError } from "./errors.ts";
import { ajv, committedSchema, sharedFixture } from "./schemas.ts";
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

/** The paths of the Proving ground's own routes, served on its bound address rather than on a page family's host. */
export interface ProvingGroundRoutes {
  /** `GET`: the task list. */
  readonly tasks: string;
  /** `POST`: judges a tool's result for one task. */
  readonly oracle: string;
  /** `PUT`: the time step its pages answer at from now on. */
  readonly time_step: string;
  /** `GET`: every request its page hosts received, and the connection record. */
  readonly observations: string;
  /** `GET`: the client certificate its mutual-TLS hosts accept. */
  readonly client_certificate: string;
  /** `GET`: that certificate's private key. */
  readonly client_key: string;
  /** `GET <path>/<size>`: the right answer of each page of the article site. */
  readonly article_answers: string;
}

const isProvingGroundRoutes: ValidateFunction<ProvingGroundRoutes> =
  ajv.compile<ProvingGroundRoutes>(committedSchema("proving-ground-routes.schema.json"));

/** The Proving ground's routes, read from the committed shared fixture. */
export function provingGroundRoutes(): ProvingGroundRoutes {
  return sharedFixture("proving-ground/routes.json", isProvingGroundRoutes);
}
