# Goodfetch Collie installer (https://goodfetch.dev/install/collie.ps1; ADRs 0090 and 0094, issue 144).
# Downloads the Windows archive and SHA256SUMS, verifies the SHA-256 digest before extracting,
# and installs `goodfetch-collie.exe`.
$ErrorActionPreference = "Stop"

$Product = "collie"
$BinName = "goodfetch-$Product"
$Repo = if ($env:GOODFETCH_REPO) { $env:GOODFETCH_REPO } else { "goodfetch/goodfetch" }
$InstallDir = if ($env:GOODFETCH_INSTALL_DIR) { $env:GOODFETCH_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Goodfetch\bin" }

$Arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
$Target = switch ($Arch) {
    "X64"   { "x86_64-pc-windows-msvc" }
    "Arm64" { "aarch64-pc-windows-msvc" }
    default { throw "Unsupported Windows architecture: $Arch" }
}

$Archive = "$BinName-$Target.tar.xz"
if ($env:GOODFETCH_RELEASE_BASE) {
    $Base = $env:GOODFETCH_RELEASE_BASE.TrimEnd("/")
} elseif ($env:GOODFETCH_VERSION) {
    $Version = $env:GOODFETCH_VERSION.TrimStart("v")
    $Base = "https://github.com/$Repo/releases/download/v$Version"
} else {
    $Base = "https://github.com/$Repo/releases/latest/download"
}

$Scratch = Join-Path ([System.IO.Path]::GetTempPath()) ("goodfetch-install-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $Scratch | Out-Null
try {
    $SumsPath = Join-Path $Scratch "SHA256SUMS"
    $ArchivePath = Join-Path $Scratch $Archive

    Write-Host "Downloading $Archive from $Base..."
    Invoke-WebRequest -Uri "$Base/SHA256SUMS" -OutFile $SumsPath -UseBasicParsing
    Invoke-WebRequest -Uri "$Base/$Archive" -OutFile $ArchivePath -UseBasicParsing

    $Expected = $null
    foreach ($Line in Get-Content -Path $SumsPath) {
        if ($Line -match '^([0-9a-fA-F]{64})\s+\*?(.+)$' -and $Matches[2].Trim() -eq $Archive) {
            $Expected = $Matches[1].ToLowerInvariant()
            break
        }
    }
    if (-not $Expected) {
        throw "$Archive not listed in SHA256SUMS"
    }

    $Actual = (Get-FileHash -Path $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($Actual -ne $Expected) {
        throw "Checksum mismatch for $Archive (expected $Expected, got $Actual)"
    }

    tar -xJf $ArchivePath -C $Scratch
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Copy-Item -Path (Join-Path $Scratch "$BinName.exe") -Destination (Join-Path $InstallDir "$BinName.exe") -Force
    Write-Host "Installed $BinName.exe to $(Join-Path $InstallDir "$BinName.exe")"
    Write-Host "Optional: verify provenance with: gh attestation verify $Archive --repo $Repo"
} finally {
    Remove-Item -Path $Scratch -Recurse -Force -ErrorAction SilentlyContinue
}
