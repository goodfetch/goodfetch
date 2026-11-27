# Signs Windows x64 and arm64 executables on a Windows x64 runner through Azure
# Artifact Signing using OIDC federation and RFC 3161 timestamps (ADRs 0039,
# 0074 and 0094; issue 142):
# - One Artifact Signing call per architecture signs both Products' `.exe` files
#   together (plus shared helpers and first-party Plugins for that architecture).
# - Signs Goodfetch Chromium Windows files fetched by digest (never rebuilds).
#
# Usage: pwsh ci/release/sign-windows.ps1 -Mode <proving-ground|artifacts> -Dir <dir> [-ChromiumDir <dir>]
param(
  [Parameter(Mandatory = $true)]
  [ValidateSet('proving-ground', 'artifacts')]
  [string]$Mode,

  [Parameter(Mandatory = $true)]
  [string]$Dir,

  [string]$ChromiumDir = ''
)

$ErrorActionPreference = 'Stop'

$endpoint = $env:AZURE_SIGNING_ENDPOINT
$account = $env:AZURE_SIGNING_ACCOUNT
$profile = $env:AZURE_SIGNING_PROFILE
$timestampServer = 'http://timestamp.acs.microsoft.com'

if (-not $endpoint -or -not $account -or -not $profile) {
  throw 'Missing AZURE_SIGNING_ENDPOINT, AZURE_SIGNING_ACCOUNT, or AZURE_SIGNING_PROFILE'
}

$metadata = @{
  Endpoint               = $endpoint
  CodeSigningAccountName = $account
  CertificateProfileName = $profile
} | ConvertTo-Json -Compress

$metadataPath = Join-Path $env:RUNNER_TEMP 'goodfetch-artifact-signing.json'
Set-Content -Path $metadataPath -Value $metadata -NoNewline

function Invoke-ArtifactSigning {
  param([string[]]$Files)
  if ($Files.Count -eq 0) { return }
  # One signtool / Artifact Signing call per architecture for all `.exe` files (ADR 0094).
  & signtool.exe sign `
    /v /fd SHA256 `
    /tr $timestampServer /td SHA256 `
    /dlib "$env:AZURE_SIGNING_DLIB" `
    /dmdf $metadataPath `
    @Files
  if ($LASTEXITCODE -ne 0) {
    throw "Azure Artifact Signing failed with exit code $LASTEXITCODE"
  }
}

foreach ($target in @('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')) {
  $targetPath = Join-Path $Dir $target
  if (-not (Test-Path $targetPath)) { continue }

  if ($Mode -eq 'proving-ground') {
    $pg = Join-Path $targetPath 'goodfetch-proving-ground.exe'
    if (Test-Path $pg) {
      Invoke-ArtifactSigning -Files @($pg)
    }
    continue
  }

  $toSign = @()
  foreach ($exe in @('goodfetch-collie.exe', 'goodfetch-foxhound.exe', 'goodfetch-filter-broker.exe')) {
    $candidate = Join-Path $targetPath $exe
    if (Test-Path $candidate) {
      $toSign += $candidate
    }
  }
  $plugins = Get-ChildItem -Path $targetPath -Filter 'goodfetch-plugin-*.exe' -File -ErrorAction SilentlyContinue
  foreach ($plugin in $plugins) {
    $toSign += $plugin.FullName
  }
  Invoke-ArtifactSigning -Files $toSign
}

# Sign fetched Goodfetch Chromium executables/DLLs by digest (never rebuild, ADR 0074).
if ($Mode -eq 'artifacts' -and $ChromiumDir -and (Test-Path $ChromiumDir)) {
  $chromiumFiles = Get-ChildItem -Path $ChromiumDir -Recurse -Include '*.exe', '*.dll' -File |
    Select-Object -ExpandProperty FullName
  if ($chromiumFiles.Count -gt 0) {
    Invoke-ArtifactSigning -Files $chromiumFiles
  }
}
