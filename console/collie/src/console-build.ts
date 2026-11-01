// What a Product's build entry composes the shell with (ADR 0087; spec
// #593): the Product the build is, which names it in the page's title and
// first screen, and the views it carries beyond those every run has, each
// shown in a tab of a run's page for a run that needs Foxhound. Collie's
// build carries none; Foxhound's build entry, in `@goodfetch/console-foxhound`,
// passes Foxhound's, so Collie's screens never import Foxhound's.
import type { Product } from "@goodfetch/collie-client/generated";
import type { MessageDescriptor } from "@lingui/core";
import type { ReactNode } from "react";
import type { ConsoleClient } from "#client/client.ts";
import type { OfflineCache } from "#offline/offline.ts";

/** What a run's view shows over: the Console's Client, the run's Run ID and the page's offline cache. */
export interface RunViewProps {
  readonly client: ConsoleClient;
  readonly runId: string;
  /** The browser's offline cache of the page's sign-in; none while `console.offline_cache` is off. */
  readonly offline?: OfflineCache | undefined;
}

/** A view of a run that needs Foxhound, which a build carries beyond those every run has. */
export interface RunView {
  /** Its tab's value, unique among the build's views. */
  readonly id: string;
  /** Its tab's name. */
  readonly name: MessageDescriptor;
  /** What its tab shows. */
  readonly content: (props: RunViewProps) => ReactNode;
}

/** A Product's Console build, as its entry composes the shell. */
export interface ConsoleBuild {
  /** The Product the build is, which its page's title and first screen name. */
  readonly product: Product;
  /** The views of a run that needs Foxhound the build carries, in their tabs' order; none in Collie's. */
  readonly runViews: readonly RunView[];
}

/** Collie's build: Collie's screens alone. */
export const COLLIE_BUILD: ConsoleBuild = { product: "collie", runViews: [] };
