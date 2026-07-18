#!/usr/bin/env pwsh
# Tests for scripts/install/install.ps1.
#
# These run on any PowerShell (Windows PowerShell 5.1 or `pwsh` on
# macOS/Linux/Windows):
#
#     pwsh -NoProfile -File scripts/install/test_install_ps1.ps1
#
# The architecture-detection cases exercise the *real* block extracted from
# install.ps1 (not a copy), only swapping the RuntimeInformation.OSArchitecture
# access for a probe we control, so we can simulate hosts where that property is
# unavailable (older Windows PowerShell / .NET Framework < 4.7.1) without needing
# such a host. See issue #1821.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$installPath = Join-Path $repoRoot 'scripts/install/install.ps1'
$script:failures = 0

function Assert-Equal {
    param([string]$Name, $Actual, $Expected)
    if ($Actual -eq $Expected) {
        Write-Host "PASS: $Name -> $Actual"
    } else {
        Write-Host "FAIL: $Name -> got '$Actual', expected '$Expected'"
        $script:failures++
    }
}

# --- 1. The real installer parses without syntax errors -----------------------
$parseErrors = $null
[System.Management.Automation.Language.Parser]::ParseFile(
    $installPath, [ref]$null, [ref]$parseErrors) | Out-Null
if ($parseErrors -and $parseErrors.Count -gt 0) {
    Write-Host "FAIL: install.ps1 has $($parseErrors.Count) parse error(s):"
    $parseErrors | ForEach-Object { Write-Host "   $($_.Message)" }
    $script:failures++
} else {
    Write-Host "PASS: install.ps1 parses cleanly"
}

# --- 2. Extract the real architecture-detection block -------------------------
# Everything from `$architecture = $null` up to (but not including) the next
# section (`$codexHome = ...`). This covers the OSArchitecture probe, the
# environment-variable fallback, and the target `switch`.
$installText = Get-Content -Raw -LiteralPath $installPath
if ($installText -notmatch '(?s)(\$architecture = \$null.*?)\r?\n\$codexHome ') {
    Write-Host 'FAIL: could not locate the architecture-detection block in install.ps1'
    exit 1
}
$archBlock = $Matches[1]

# Swap the real OSArchitecture access for a probe scriptblock we control, so we
# can make it succeed (modern hosts) or throw (old .NET) at will.
$probeMarker = '[System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture'
if (-not $archBlock.Contains($probeMarker)) {
    Write-Host 'FAIL: OSArchitecture access not found in the extracted block'
    exit 1
}
$archBlock = $archBlock.Replace($probeMarker, '(& $script:OSArchitectureProbe)')

function Resolve-InstallerArchitecture {
    param(
        [scriptblock]$OSArchitectureProbe,  # returns arch string, or throws
        [string]$ArchEW6432 = '',           # $env:PROCESSOR_ARCHITEW6432
        [string]$ArchProc = ''              # $env:PROCESSOR_ARCHITECTURE
    )
    $script:OSArchitectureProbe = $OSArchitectureProbe
    $env:PROCESSOR_ARCHITEW6432 = $ArchEW6432
    $env:PROCESSOR_ARCHITECTURE = $ArchProc
    $architecture = $null
    $target = $null
    Invoke-Expression $archBlock
    [pscustomobject]@{ Architecture = $architecture; Target = $target }
}

$throws = { throw "The property 'OSArchitecture' cannot be found on this object." }

# --- 3. Modern hosts: OSArchitecture works -> behavior unchanged --------------
$r = Resolve-InstallerArchitecture -OSArchitectureProbe { 'X64' }
Assert-Equal 'modern x64 target' $r.Target 'x86_64-pc-windows-msvc'
$r = Resolve-InstallerArchitecture -OSArchitectureProbe { 'Arm64' }
Assert-Equal 'modern arm64 target' $r.Target 'aarch64-pc-windows-msvc'

# --- 4. Old .NET (#1821): OSArchitecture throws -> env-var fallback -----------
$r = Resolve-InstallerArchitecture -OSArchitectureProbe $throws -ArchProc 'AMD64'
Assert-Equal 'old .NET amd64 target' $r.Target 'x86_64-pc-windows-msvc'
$r = Resolve-InstallerArchitecture -OSArchitectureProbe $throws -ArchProc 'ARM64'
Assert-Equal 'old .NET arm64 target' $r.Target 'aarch64-pc-windows-msvc'

# --- 5. WOW64: 32-bit PowerShell on 64-bit OS -> real arch in ARCHITEW6432 ----
$r = Resolve-InstallerArchitecture -OSArchitectureProbe $throws -ArchEW6432 'ARM64' -ArchProc 'x86'
Assert-Equal 'old .NET WOW64 arm64 target' $r.Target 'aarch64-pc-windows-msvc'
$r = Resolve-InstallerArchitecture -OSArchitectureProbe $throws -ArchEW6432 'AMD64' -ArchProc 'x86'
Assert-Equal 'old .NET WOW64 amd64 target' $r.Target 'x86_64-pc-windows-msvc'

if ($script:failures -eq 0) {
    Write-Host "`nAll install.ps1 checks passed."
    exit 0
} else {
    Write-Host "`n$($script:failures) install.ps1 check(s) failed."
    exit 1
}
