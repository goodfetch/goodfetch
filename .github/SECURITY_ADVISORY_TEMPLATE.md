# Goodfetch Security Advisory Template

> Use this template when publishing a GitHub Security Advisory for a security
> fix in Goodfetch Collie or Goodfetch Foxhound (ADR 0030, issue 144).
> Request a CVE through GitHub Security Advisories before publishing.

## Summary

- **CVE:** `CVE-YYYY-NNNNN` (requested via GitHub Security Advisories)
- **Products affected:** Goodfetch Collie (`goodfetch-collie`, `@goodfetch/collie`, `@goodfetch/collie-client`) / Goodfetch Foxhound (`goodfetch-foxhound`, `@goodfetch/foxhound`)
- **Affected versions:** `< x.y.z`
- **Patched versions:** `>= x.y.z` (and `>= prev.y.z` for the previous major's supported minor, if applicable)
- **Severity:** Critical / High / Medium / Low (CVSS 4.0 vector)

## Impact

Describe the vulnerability, the trust boundary it affects (ADR 0067), and who
is impacted.

## Remediation

Upgrade both Products to the patched Product version. If a persisted format
changed across a minor, run `upgrade-files` before starting the new version.

## Package-manager release-age exclusions (hotfix)

The authoritative, CI-tested exclusion snippets live in
[`docs/release-age-exclusions.toml`](../docs/release-age-exclusions.toml)
(ADRs 0030, 0069 and 0090). Copy the snippet for your package manager to let
the hotfix through a release-age gate, and remove it once the release is a day
old:

- **npm** (`.npmrc`): `min-release-age-exclude[]=@goodfetch/*`
- **pnpm** (`pnpm-workspace.yaml`): `minimumReleaseAgeExclude: ["@goodfetch/*"]`
- **Yarn** (`.yarnrc.yml`): `npmPreapprovedPackages: ["@goodfetch/*"]`
- **Bun** (`bunfig.toml`): `minimumReleaseAgeExcludes` listing every `@goodfetch/*` root, client and platform package
- **Deno** (`deno.json`): `minimumDependencyAge.exclude` for `npm:@goodfetch/*`
- **uv** (`pyproject.toml`): `exclude-newer-package = { goodfetch-collie = false, goodfetch-foxhound = false, goodfetch-collie-client = false }`

## Verification

Verify the release archive provenance with GitHub artifact attestations:

```sh
gh attestation verify goodfetch-collie-<target>.tar.xz --repo goodfetch/goodfetch
gh attestation verify goodfetch-foxhound-<target>.tar.xz --repo goodfetch/goodfetch
```

## Credits

Acknowledge the reporter (with permission).
