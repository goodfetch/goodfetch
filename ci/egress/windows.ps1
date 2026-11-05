# ADR 0043's outbound-traffic deny on a GitHub-hosted Windows runner, called by
# ci/egress/egress.sh: windows.ps1 deny|canary|check <workflow file>#<job id>.
#
# The runner's own agent must keep its connection, so nothing blocks the whole
# machine. Instead, Windows Firewall rules block the product's programs (every
# executable under the workspace, the build's tests and xtask included, and
# the test runtimes Node, Bun, Deno and the CPython builds) from every address
# but loopback and the job's allowed endpoints, as `xtask egress rules
# --windows` writes them. The firewall logs each dropped packet; that log,
# from the line where the deny began, is the record the canary's verdict and
# the end-of-job check read. A program the tests create after the deny, outside
# these folders, is not blocked: the gap is recorded in TESTING.md.
param(
    [Parameter(Mandatory = $true)][ValidateSet('deny', 'canary', 'check')][string] $Command,
    [Parameter(Mandatory = $true)][string] $Job
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$files = Join-Path $root 'target\egress'
$log = Join-Path $env:SystemRoot 'System32\LogFiles\Firewall\pfirewall.log'
$group = 'goodfetch-egress'

function Invoke-Xtask {
    param([string[]] $Arguments)
    Push-Location $root
    try {
        & cargo run --quiet --locked --bin xtask -- egress @Arguments
        if ($LASTEXITCODE -ne 0) { throw "xtask egress $($Arguments -join ' ') exited with $LASTEXITCODE" }
    }
    finally { Pop-Location }
}

# The firewall log's lines written since the deny began, copied where xtask reads them.
function Copy-NewLog {
    param([string] $Name)
    $start = [int](Get-Content (Join-Path $files 'log-start.txt'))
    $lines = @(Get-Content -Path $log -ErrorAction SilentlyContinue)
    $header = $lines | Where-Object { $_ -like '#*' }
    $new = if ($lines.Count -gt $start) { $lines[$start..($lines.Count - 1)] } else { @() }
    @($header) + @($new | Where-Object { $_ -notlike '#*' }) | Set-Content -Path (Join-Path $files $Name) -Encoding ascii
}

function Invoke-Deny {
    New-Item -ItemType Directory -Force -Path $files | Out-Null
    $endpoints = Invoke-Xtask @('endpoints', '--job', $Job)
    $resolved = foreach ($line in $endpoints) {
        $hostName, $port = $line -split ' '
        try {
            Resolve-DnsName -Name $hostName -Type A -ErrorAction Stop |
                Where-Object { $_.Type -eq 'A' } |
                ForEach-Object { "$hostName $port $($_.IPAddress)" }
        }
        catch { Write-Warning "$hostName did not resolve, so the job cannot reach it" }
    }
    $resolved | Sort-Object -Unique | Set-Content -Path (Join-Path $files 'resolved.txt') -Encoding ascii
    $rules = Invoke-Xtask @('rules', '--job', $Job, '--resolved', 'target/egress/resolved.txt', '--windows')

    $programs = @(Get-ChildItem -Path $root -Recurse -File -Filter '*.exe' -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -notlike (Join-Path $root '.git\*') } |
            ForEach-Object { $_.FullName })
    foreach ($runtime in 'node', 'bun', 'deno', 'python', 'pnpm', 'uv') {
        $programs += @(Get-Command $runtime -All -CommandType Application -ErrorAction SilentlyContinue |
                ForEach-Object { $_.Source })
    }
    foreach ($entry in ($env:GOODFETCH_NODE_LINES -split ' ' | Where-Object { $_ })) {
        $programs += Join-Path ($entry -split '=', 2)[1] 'node.exe'
    }
    $uvPythons = & uv python dir
    $programs += @(Get-ChildItem -Path $uvPythons -Recurse -File -Filter 'python*.exe' -ErrorAction SilentlyContinue |
            ForEach-Object { $_.FullName })
    $programs = $programs | Where-Object { $_ -like '*.exe' -and (Test-Path $_) } |
        ForEach-Object { (Resolve-Path $_).Path } | Sort-Object -Unique

    Set-NetFirewallProfile -All -Enabled True -LogBlocked True -LogFileName $log -LogMaxSizeKilobytes 32767
    $start = if (Test-Path $log) { @(Get-Content -Path $log).Count } else { 0 }
    Set-Content -Path (Join-Path $files 'log-start.txt') -Value $start -Encoding ascii

    foreach ($program in $programs) {
        foreach ($rule in $rules) {
            $protocol, $addresses, $ports = $rule -split ' '
            $arguments = @{
                DisplayName   = "$group $program"
                Group         = $group
                Direction     = 'Outbound'
                Action        = 'Block'
                Program       = $program
                Protocol      = $protocol
                RemoteAddress = $addresses -split ','
            }
            if ($ports -ne '*') { $arguments.RemotePort = $ports -split ',' }
            New-NetFirewallRule @arguments | Out-Null
        }
    }
    Set-Content -Path (Join-Path $files 'denied') -Value $Job -Encoding ascii
    Write-Output "Outbound traffic denied for $Job, $($programs.Count) programs, but to $(@($endpoints).Count) allowed endpoints."
}

function Invoke-Canary {
    Push-Location $root
    try {
        & cargo run --quiet --locked --offline --bin xtask -- egress canary attempt --outcome target/egress/canary-outcome.txt
        if ($LASTEXITCODE -ne 0) { throw "the canary's attempt exited with $LASTEXITCODE" }
    }
    finally { Pop-Location }
    # The firewall writes its log in the background: wait for the canary's
    # drop to reach it, then judge. A wait for a record is not a retry.
    $canary = (Get-Content (Join-Path $root 'ci\egress\endpoints.toml') |
            Where-Object { $_ -match '^canary = "(.*):(\d+)"' } |
            ForEach-Object { $Matches[1] })
    for ($second = 0; $second -lt 30; $second++) {
        Copy-NewLog 'canary.log'
        if (Select-String -Path (Join-Path $files 'canary.log') -SimpleMatch " $canary " -Quiet) { break }
        Start-Sleep -Seconds 1
    }
    Invoke-Xtask @('canary', 'verdict', '--outcome', 'target/egress/canary-outcome.txt', '--firewall-log', 'target/egress/canary.log')
    Write-Output 'The Egress canary was refused.'
}

function Invoke-Check {
    if (-not (Test-Path (Join-Path $files 'denied'))) {
        Write-Output 'The deny never started, so no step ran under it.'
        return
    }
    Copy-NewLog 'firewall.log'
    Invoke-Xtask @('report', '--firewall-log', 'target/egress/firewall.log')
    Write-Output 'The deny refused nothing but the Egress canary.'
}

switch ($Command) {
    'deny' { Invoke-Deny }
    'canary' { Invoke-Canary }
    'check' { Invoke-Check }
}
