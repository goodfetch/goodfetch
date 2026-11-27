# Platform signature verifier for Windows x64 and arm64 (ADR 0039, ADR 0094;
# issue 142): runs `signtool verify /pa /tw` (verifying Authenticode chain and
# RFC 3161 timestamp) on every Windows executable and emits JSON for
# `xtask release signature-gate --report`.
#
# Usage: pwsh ci/release/verify-platform-signatures.ps1 -BinariesDir <dir> -OutJson <path>
param(
  [Parameter(Mandatory = $true)]
  [string]$BinariesDir,

  [Parameter(Mandatory = $true)]
  [string]$OutJson
)

$ErrorActionPreference = 'Stop'

$entries = @()
foreach ($target in @('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')) {
  foreach ($bin in @('goodfetch-collie.exe', 'goodfetch-foxhound.exe', 'goodfetch-proving-ground.exe', 'goodfetch-filter-broker.exe')) {
    $path = Join-Path (Join-Path $BinariesDir $target) $bin
    if (-not (Test-Path $path)) { continue }

    & signtool.exe verify /pa "$path" | Out-Null
    $paOk = ($LASTEXITCODE -eq 0)

    & signtool.exe verify /pa /tw "$path" | Out-Null
    $tsOk = ($LASTEXITCODE -eq 0)

    $entries += @{
      target            = $target
      binary            = $bin
      signtool_pa       = $paOk
      rfc3161_timestamp = $tsOk
    }
  }
}

$outParent = Split-Path -Parent $OutJson
if ($outParent) {
  New-Item -ItemType Directory -Force -Path $outParent | Out-Null
}
@{ windows = $entries } | ConvertTo-Json -Depth 4 | Set-Content -Path $OutJson
