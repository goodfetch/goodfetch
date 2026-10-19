import { describe, expect, test } from "vitest";
import { refuseBypassCSP, type CspOptions } from "./bypass-csp.ts";

describe("refuseBypassCSP", () => {
  test.for<[string, CspOptions]>([
    ["bypassCSP", { bypassCSP: true }],
    ["contextOptions' bypassCSP", { contextOptions: { bypassCSP: true } }],
  ])("refuses %s with bypass_csp_refused", ([, options]) => {
    const refusal = () => refuseBypassCSP(options);

    expect(refusal).toThrow(expect.objectContaining({ code: "bypass_csp_refused" }));
  });

  test.for<[string, CspOptions]>([
    ["no option", {}],
    ["bypassCSP off", { bypassCSP: false, contextOptions: { bypassCSP: false } }],
  ])("allows %s", ([, options]) => {
    const refusal = () => refuseBypassCSP(options);

    expect(refusal).not.toThrow();
  });
});
