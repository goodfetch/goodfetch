// The types of the module agent-browser's own launcher finds its native
// binary with (issue 2442). The package ships no types, and act.ts runs that
// binary itself, so the trial's process tree holds no launcher of its own.
declare module "agent-browser/scripts/native.js" {
  /** Where the binary for this host can live. */
  export interface NativeTarget {
    readonly packageName: string;
    readonly file: string;
    readonly binaryName: string;
    readonly candidates: readonly string[];
  }

  /** This host's target, or none when agent-browser ships no binary for it. */
  export function nativeTarget(): NativeTarget | null;

  /** The binary to run, from the package's `bin/` or its platform package. */
  export function findNativeBinary(packageRoot: string, target: NativeTarget): string | null;
}
