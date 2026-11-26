// Foxhound's values of the types both Products share (ADR 0095): the
// package's root names each shared type by Foxhound's widened union, so a
// Foxhound program's `BlockCause` knows `decoy`, its `UsageKind`
// `paid_fetch` and its `LimitScope` `partitioned_crawl`, where the Collie
// client package it re-exports knows them only as strings it keeps
// (ADR 0079). Python's names live in `goodfetch_foxhound.generated`, as
// its package root re-exports none of the client package's types.
import type * as Collie from "@goodfetch/collie-client/generated";
import { expectTypeOf } from "vitest";
import type * as Widened from "../src/generated/shared-types.ts";
import type {
  AuditKind,
  BlockCause,
  CompletenessReasonCode,
  LimitScope,
  RunWarningCode,
  UsageKind,
} from "../src/index.ts";

expectTypeOf<BlockCause>().toEqualTypeOf<Widened.BlockCause>();
expectTypeOf<UsageKind>().toEqualTypeOf<Widened.UsageKind>();
expectTypeOf<LimitScope>().toEqualTypeOf<Widened.LimitScope>();
expectTypeOf<AuditKind>().toEqualTypeOf<Widened.AuditKind>();
expectTypeOf<RunWarningCode>().toEqualTypeOf<Widened.RunWarningCode>();
expectTypeOf<CompletenessReasonCode>().toEqualTypeOf<Widened.CompletenessReasonCode>();

// Collie's own union keeps Foxhound's values as strings, never as members it names.
expectTypeOf<BlockCause>().not.toEqualTypeOf<Collie.BlockCause>();
expectTypeOf<UsageKind>().not.toEqualTypeOf<Collie.UsageKind>();
expectTypeOf<LimitScope>().not.toEqualTypeOf<Collie.LimitScope>();
