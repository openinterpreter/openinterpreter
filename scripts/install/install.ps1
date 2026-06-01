param(
    [string]$Release = "latest",
    [string]$Repo = $(if ([string]::IsNullOrWhiteSpace($env:OPEN_INTERPRETER_GITHUB_REPO)) { "KillianLucas/oix" } else { $env:OPEN_INTERPRETER_GITHUB_REPO })
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Write-Step {
    param(
        [string]$Message
    )

    Write-Host "==> $Message"
}

function Write-WarningStep {
    param(
        [string]$Message
    )

    Write-Warning $Message
}

function Prompt-YesNo {
    param(
        [string]$Prompt
    )

    # Non-interactive callers (notably the in-app auto-updater) must never block
    # on a prompt; honor an explicit opt-out and default to "no".
    if (-not [string]::IsNullOrWhiteSpace($env:OPEN_INTERPRETER_NONINTERACTIVE)) {
        return $false
    }

    if ([Console]::IsInputRedirected -or [Console]::IsOutputRedirected) {
        return $false
    }

    $choice = Read-Host "$Prompt [y/N]"
    return $choice -match "^(?i:y(?:es)?)$"
}

function Normalize-Version {
    param(
        [string]$RawVersion
    )

    if ([string]::IsNullOrWhiteSpace($RawVersion) -or $RawVersion -eq "latest") {
        return "latest"
    }

    if ($RawVersion.StartsWith("v")) {
        return $RawVersion.Substring(1)
    }

    return $RawVersion
}

function Get-GitHubHeaders {
    param(
        [string]$Accept = "application/vnd.github+json"
    )

    $headers = @{
        "Accept" = $Accept
        "X-GitHub-Api-Version" = "2022-11-28"
    }
    $token = if (-not [string]::IsNullOrWhiteSpace($env:GITHUB_TOKEN)) {
        $env:GITHUB_TOKEN
    } elseif (-not [string]::IsNullOrWhiteSpace($env:GH_TOKEN)) {
        $env:GH_TOKEN
    } else {
        $null
    }
    if (-not [string]::IsNullOrWhiteSpace($token)) {
        $headers["Authorization"] = "Bearer $token"
    }
    return $headers
}

function Invoke-GitHubJson {
    param(
        [string]$Uri
    )

    return Invoke-RestMethod -Uri $Uri -Headers (Get-GitHubHeaders)
}

function Invoke-GitHubAssetDownload {
    param(
        [string]$Uri,
        [string]$OutFile
    )

    $previousProgressPreference = $ProgressPreference
    try {
        $global:ProgressPreference = "Continue"
        Invoke-WebRequest -Uri $Uri -Headers (Get-GitHubHeaders -Accept "application/octet-stream") -OutFile $OutFile
    } finally {
        $global:ProgressPreference = $previousProgressPreference
    }
}

function Invoke-GitHubAssetText {
    param(
        [string]$Uri
    )

    return Invoke-RestMethod -Uri $Uri -Headers (Get-GitHubHeaders -Accept "application/octet-stream")
}

function Get-ReleaseAssetMetadata {
    param(
        [string]$AssetName,
        [string]$ResolvedVersion
    )

    $release = Invoke-GitHubJson -Uri "https://api.github.com/repos/$Repo/releases/tags/v$ResolvedVersion"
    $asset = $release.assets | Where-Object { $_.name -eq $AssetName } | Select-Object -First 1
    if ($null -eq $asset) {
        throw "Could not find release asset $AssetName for Open Interpreter $ResolvedVersion."
    }

    $digest = $null
    $digestMatch = [regex]::Match([string]$asset.digest, "^sha256:([0-9a-fA-F]{64})$")
    if ($digestMatch.Success) {
        $digest = $digestMatch.Groups[1].Value.ToLowerInvariant()
    } else {
        $checksumAsset = $release.assets | Where-Object { $_.name -eq "$AssetName.sha256" } | Select-Object -First 1
        if ($null -ne $checksumAsset) {
            $checksumText = [string](Invoke-GitHubAssetText -Uri $checksumAsset.url)
            $checksumMatch = [regex]::Match($checksumText, "\b([0-9a-fA-F]{64})\b")
            if ($checksumMatch.Success) {
                $digest = $checksumMatch.Groups[1].Value.ToLowerInvariant()
            }
        }
    }
    if ([string]::IsNullOrWhiteSpace($digest)) {
        throw "Could not find SHA-256 digest for release asset $AssetName."
    }

    return [PSCustomObject]@{
        Url = $asset.url
        Sha256 = $digest
    }
}

function Test-ArchiveDigest {
    param(
        [string]$ArchivePath,
        [string]$ExpectedDigest
    )

    $actualDigest = (Get-FileHash -LiteralPath $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualDigest -ne $ExpectedDigest) {
        throw "Downloaded Open Interpreter archive checksum did not match release metadata. Expected $ExpectedDigest but got $actualDigest."
    }
}

function Path-Contains {
    param(
        [string]$PathValue,
        [string]$Entry
    )

    if ([string]::IsNullOrWhiteSpace($PathValue)) {
        return $false
    }

    $needle = $Entry.TrimEnd("\")
    foreach ($segment in $PathValue.Split(";", [System.StringSplitOptions]::RemoveEmptyEntries)) {
        if ($segment.TrimEnd("\") -ieq $needle) {
            return $true
        }
    }

    return $false
}

function Invoke-WithInstallLock {
    param(
        [string]$LockPath,
        [scriptblock]$Script
    )

    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $LockPath) | Out-Null
    $lock = $null
    while ($null -eq $lock) {
        try {
            $lock = [System.IO.File]::Open(
                $LockPath,
                [System.IO.FileMode]::OpenOrCreate,
                [System.IO.FileAccess]::ReadWrite,
                [System.IO.FileShare]::None
            )
        } catch [System.IO.IOException] {
            Start-Sleep -Milliseconds 250
        }
    }
    try {
        & $Script
    } finally {
        $lock.Dispose()
    }
}

function Remove-StaleInstallArtifacts {
    param(
        [string]$ReleasesDir
    )

    if (Test-Path -LiteralPath $ReleasesDir -PathType Container) {
        Get-ChildItem -LiteralPath $ReleasesDir -Force -Directory -Filter ".staging.*" -ErrorAction SilentlyContinue |
            Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Resolve-Version {
    $normalizedVersion = Normalize-Version -RawVersion $Release
    if ($normalizedVersion -ne "latest") {
        return $normalizedVersion
    }

    try {
        $release = Invoke-GitHubJson -Uri "https://api.github.com/repos/$Repo/releases/latest"
    } catch {
        $releases = Invoke-GitHubJson -Uri "https://api.github.com/repos/$Repo/releases"
        $release = $releases | Select-Object -First 1
    }
    if (-not $release.tag_name) {
        Write-Error "Failed to resolve the latest Open Interpreter release version."
        exit 1
    }

    return (Normalize-Version -RawVersion $release.tag_name)
}

function Get-VersionFromBinary {
    param(
        [string]$InterpreterPath
    )

    if (-not (Test-Path -LiteralPath $InterpreterPath -PathType Leaf)) {
        return $null
    }

    try {
        $versionOutput = & $InterpreterPath --version 2>$null
    } catch {
        return $null
    }

    if ($versionOutput -match '([0-9][0-9A-Za-z.+-]*)$') {
        return $matches[1]
    }

    return $null
}

function Get-CurrentInstalledVersion {
    param(
        [string]$StandaloneCurrentDir
    )

    $standaloneVersion = Get-VersionFromBinary -InterpreterPath (Join-Path $StandaloneCurrentDir "interpreter.exe")
    if (-not [string]::IsNullOrWhiteSpace($standaloneVersion)) {
        return $standaloneVersion
    }

    return $null
}

function Test-OldStandaloneBinLayout {
    param(
        [string]$VisibleBinDir,
        [string]$DefaultVisibleBinDir
    )

    if (-not $VisibleBinDir.Equals($DefaultVisibleBinDir, [System.StringComparison]::OrdinalIgnoreCase)) {
        return $false
    }
    if (-not (Test-Path -LiteralPath $VisibleBinDir -PathType Container)) {
        return $false
    }

    $item = Get-Item -LiteralPath $VisibleBinDir -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        return $false
    }

    $requiredFiles = @(
        "interpreter.exe",
        "interpreter-root-tui.exe",
        "interpreter-tui.exe",
        "interpreter-app-server.exe",
        "interpreter-acp.exe"
    )
    foreach ($fileName in $requiredFiles) {
        if (-not (Test-Path -LiteralPath (Join-Path $VisibleBinDir $fileName) -PathType Leaf)) {
            return $false
        }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $VisibleBinDir "interpreter-exec.exe") -PathType Leaf)) {
        return $false
    }

    $knownFiles = @(
        "interpreter.exe",
        "interpreter-root-tui.exe",
        "interpreter-tui.exe",
        "interpreter-app-server.exe",
        "interpreter-acp.exe",
        "interpreter-exec.exe",
        "rg.exe"
    )
    foreach ($child in Get-ChildItem -LiteralPath $VisibleBinDir -Force) {
        if ($child.PSIsContainer) {
            return $false
        }
        if ($knownFiles -notcontains $child.Name) {
            return $false
        }
    }

    return $true
}

function Move-OldStandaloneBinIfApproved {
    param(
        [string]$VisibleBinDir,
        [string]$DefaultVisibleBinDir
    )

    if (-not (Test-OldStandaloneBinLayout -VisibleBinDir $VisibleBinDir -DefaultVisibleBinDir $DefaultVisibleBinDir)) {
        return $null
    }

    Write-Step "We found an older Open Interpreter install at $VisibleBinDir"
    Write-WarningStep "To continue, Open Interpreter needs to update the install at this path."
    if (-not (Prompt-YesNo "Replace it with the current Open Interpreter setup now?")) {
        throw "Cannot replace older standalone install without confirmation: $VisibleBinDir"
    }

    $backupDir = "$VisibleBinDir.backup.$([DateTimeOffset]::UtcNow.ToUnixTimeSeconds()).$PID"
    Write-Step "Moving older standalone install to $backupDir"
    Move-Item -LiteralPath $VisibleBinDir -Destination $backupDir
    return $backupDir
}

function Add-JunctionSupportType {
    if (([System.Management.Automation.PSTypeName]'OpenInterpreterInstaller.Junction').Type) {
        return
    }

    Add-Type -TypeDefinition @"
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace OpenInterpreterInstaller
{
    public static class Junction
    {
        private const uint GENERIC_WRITE = 0x40000000;
        private const uint FILE_SHARE_READ = 0x00000001;
        private const uint FILE_SHARE_WRITE = 0x00000002;
        private const uint FILE_SHARE_DELETE = 0x00000004;
        private const uint OPEN_EXISTING = 3;
        private const uint FILE_FLAG_BACKUP_SEMANTICS = 0x02000000;
        private const uint FILE_FLAG_OPEN_REPARSE_POINT = 0x00200000;
        private const uint FSCTL_SET_REPARSE_POINT = 0x000900A4;
        private const uint IO_REPARSE_TAG_MOUNT_POINT = 0xA0000003;
        private const int HeaderLength = 20;

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern SafeFileHandle CreateFileW(
            string lpFileName,
            uint dwDesiredAccess,
            uint dwShareMode,
            IntPtr lpSecurityAttributes,
            uint dwCreationDisposition,
            uint dwFlagsAndAttributes,
            IntPtr hTemplateFile);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool DeviceIoControl(
            SafeFileHandle hDevice,
            uint dwIoControlCode,
            byte[] lpInBuffer,
            int nInBufferSize,
            IntPtr lpOutBuffer,
            int nOutBufferSize,
            out int lpBytesReturned,
            IntPtr lpOverlapped);

        public static void SetTarget(string linkPath, string targetPath)
        {
            string substituteName = "\\??\\" + Path.GetFullPath(targetPath);
            byte[] substituteNameBytes = Encoding.Unicode.GetBytes(substituteName);
            if (substituteNameBytes.Length > ushort.MaxValue - HeaderLength) {
                throw new ArgumentException("Junction target path is too long.", "targetPath");
            }

            byte[] reparseBuffer = new byte[substituteNameBytes.Length + HeaderLength];
            WriteUInt32(reparseBuffer, 0, IO_REPARSE_TAG_MOUNT_POINT);
            WriteUInt16(reparseBuffer, 4, checked((ushort)(substituteNameBytes.Length + 12)));
            WriteUInt16(reparseBuffer, 8, 0);
            WriteUInt16(reparseBuffer, 10, checked((ushort)substituteNameBytes.Length));
            WriteUInt16(reparseBuffer, 12, checked((ushort)(substituteNameBytes.Length + 2)));
            WriteUInt16(reparseBuffer, 14, 0);
            Buffer.BlockCopy(substituteNameBytes, 0, reparseBuffer, 16, substituteNameBytes.Length);

            using (SafeFileHandle handle = CreateFileW(
                linkPath,
                GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                IntPtr.Zero,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                IntPtr.Zero))
            {
                if (handle.IsInvalid) {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }

                int bytesReturned;
                if (!DeviceIoControl(
                    handle,
                    FSCTL_SET_REPARSE_POINT,
                    reparseBuffer,
                    reparseBuffer.Length,
                    IntPtr.Zero,
                    0,
                    out bytesReturned,
                    IntPtr.Zero))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }
            }
        }

        private static void WriteUInt16(byte[] buffer, int offset, ushort value)
        {
            buffer[offset] = (byte)value;
            buffer[offset + 1] = (byte)(value >> 8);
        }

        private static void WriteUInt32(byte[] buffer, int offset, uint value)
        {
            buffer[offset] = (byte)value;
            buffer[offset + 1] = (byte)(value >> 8);
            buffer[offset + 2] = (byte)(value >> 16);
            buffer[offset + 3] = (byte)(value >> 24);
        }
    }
}
"@
}

function Set-JunctionTarget {
    param(
        [string]$LinkPath,
        [string]$TargetPath
    )

    Add-JunctionSupportType
    [OpenInterpreterInstaller.Junction]::SetTarget($LinkPath, $TargetPath)
}

function Test-IsJunction {
    param(
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        return $false
    }

    $item = Get-Item -LiteralPath $Path -Force
    return ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -and $item.LinkType -eq "Junction"
}

function Ensure-Junction {
    param(
        [string]$LinkPath,
        [string]$TargetPath,
        [string]$InstallerOwnedTargetPrefix
    )

    if (-not (Test-Path -LiteralPath $LinkPath)) {
        New-Item -ItemType Junction -Path $LinkPath -Target $TargetPath | Out-Null
        return
    }

    $item = Get-Item -LiteralPath $LinkPath -Force
    if (Test-IsJunction -Path $LinkPath) {
        $existingTarget = [string]$item.Target
        if (-not [string]::IsNullOrWhiteSpace($InstallerOwnedTargetPrefix)) {
            $ownedTargetPrefix = $InstallerOwnedTargetPrefix.TrimEnd("\\")
            if (-not $existingTarget.StartsWith($ownedTargetPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
                throw "Refusing to retarget junction at $LinkPath because it is not managed by this installer."
            }
        }
        if ($existingTarget.Equals($TargetPath, [System.StringComparison]::OrdinalIgnoreCase)) {
            return
        }

        # Keep the path itself in place and only retarget the junction. That
        # avoids a gap where current or the visible bin path disappears during
        # an update.
        Set-JunctionTarget -LinkPath $LinkPath -TargetPath $TargetPath
        return
    }

    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "Refusing to replace non-junction reparse point at $LinkPath."
    }

    if ($item.PSIsContainer) {
        if ((Get-ChildItem -LiteralPath $LinkPath -Force | Select-Object -First 1) -ne $null) {
            throw "Refusing to replace non-empty directory at $LinkPath with a junction."
        }

        Remove-Item -LiteralPath $LinkPath -Force
        New-Item -ItemType Junction -Path $LinkPath -Target $TargetPath | Out-Null
        return
    }

    throw "Refusing to replace file at $LinkPath with a junction."
}

function Test-ReleaseIsComplete {
    param(
        [string]$ReleaseDir,
        [string]$ExpectedVersion,
        [string]$ExpectedTarget
    )

    if (-not (Test-Path -LiteralPath $ReleaseDir -PathType Container)) {
        return $false
    }

    $expectedFiles = @(
        "interpreter.exe",
        "interpreter-root-tui.exe",
        "interpreter-tui.exe",
        "interpreter-app-server.exe",
        "interpreter-acp.exe",
        "interpreter-exec.exe"
    )
    foreach ($name in $expectedFiles) {
        if (-not (Test-Path -LiteralPath (Join-Path $ReleaseDir $name) -PathType Leaf)) {
            return $false
        }
    }

    return (Split-Path -Leaf $ReleaseDir) -eq "$ExpectedVersion-$ExpectedTarget"
}

function Test-VisibleInterpreterCommand {
    param(
        [string]$VisibleBinDir
    )

    $interpreterCommand = Join-Path $VisibleBinDir "interpreter.exe"
    & $interpreterCommand --version *> $null
    if ($LASTEXITCODE -ne 0) {
        throw "Installed Open Interpreter command failed verification: $interpreterCommand --version"
    }
}

function Get-ExistingInterpreterCommand {
    $existing = Get-Command interpreter -ErrorAction SilentlyContinue
    if ($null -eq $existing) {
        return $null
    }

    return $existing.Source
}

if ($env:OS -ne "Windows_NT") {
    Write-Error "install.ps1 supports Windows only. Use install.sh on macOS or Linux."
    exit 1
}

if (-not [Environment]::Is64BitOperatingSystem) {
    Write-Error "Open Interpreter requires a 64-bit version of Windows."
    exit 1
}

$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
$target = $null
$platformLabel = $null
switch ($architecture) {
    "Arm64" {
        $target = "aarch64-pc-windows-msvc"
        $platformLabel = "Windows (ARM64)"
    }
    "X64" {
        $target = "x86_64-pc-windows-msvc"
        $platformLabel = "Windows (x64)"
    }
    default {
        Write-Error "Unsupported architecture: $architecture"
        exit 1
    }
}

$openInterpreterHome = if ([string]::IsNullOrWhiteSpace($env:OPEN_INTERPRETER_HOME)) {
    Join-Path $env:USERPROFILE ".openinterpreter"
} else {
    $env:OPEN_INTERPRETER_HOME
}
$standaloneRoot = Join-Path $openInterpreterHome "packages\standalone"
$releasesDir = Join-Path $standaloneRoot "releases"
$currentDir = Join-Path $standaloneRoot "current"
$lockPath = Join-Path $standaloneRoot "install.lock"

$defaultVisibleBinDir = Join-Path $env:LOCALAPPDATA "Programs\Open Interpreter\bin"
if ([string]::IsNullOrWhiteSpace($env:OPEN_INTERPRETER_INSTALL_DIR)) {
    $visibleBinDir = $defaultVisibleBinDir
} else {
    $visibleBinDir = $env:OPEN_INTERPRETER_INSTALL_DIR
}

$currentVersion = Get-CurrentInstalledVersion -StandaloneCurrentDir $currentDir
$resolvedVersion = Resolve-Version
$releaseName = "$resolvedVersion-$target"
$releaseDir = Join-Path $releasesDir $releaseName

if (-not [string]::IsNullOrWhiteSpace($currentVersion) -and $currentVersion -ne $resolvedVersion) {
    Write-Step "Updating Open Interpreter from $currentVersion to $resolvedVersion"
} elseif (-not [string]::IsNullOrWhiteSpace($currentVersion)) {
    Write-Step "Refreshing Open Interpreter $currentVersion"
} else {
    Write-Step "Installing Open Interpreter"
}
Write-Step "Detected platform: $platformLabel"
Write-Step "Resolved version: $resolvedVersion"

$oldStandaloneBackup = $null

$packageAsset = "open-interpreter-$target.tar.gz"
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("open-interpreter-install-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $tempDir | Out-Null

try {
    Invoke-WithInstallLock -LockPath $lockPath -Script {
        Remove-StaleInstallArtifacts -ReleasesDir $releasesDir

        if (-not (Test-ReleaseIsComplete -ReleaseDir $releaseDir -ExpectedVersion $resolvedVersion -ExpectedTarget $target)) {
            if (Test-Path -LiteralPath $releaseDir) {
                Write-WarningStep "Found incomplete existing release at $releaseDir. Reinstalling."
            }

            $archivePath = Join-Path $tempDir $packageAsset
            $extractDir = Join-Path $tempDir "extract"
            $stagingDir = Join-Path $releasesDir ".staging.$releaseName.$PID"
            $assetMetadata = Get-ReleaseAssetMetadata -AssetName $packageAsset -ResolvedVersion $resolvedVersion

            Write-Step "Downloading Open Interpreter"
            Invoke-GitHubAssetDownload -Uri $assetMetadata.Url -OutFile $archivePath
            Test-ArchiveDigest -ArchivePath $archivePath -ExpectedDigest $assetMetadata.Sha256

            New-Item -ItemType Directory -Force -Path $extractDir | Out-Null
            New-Item -ItemType Directory -Force -Path $releasesDir | Out-Null
            if (Test-Path -LiteralPath $stagingDir) {
                Remove-Item -LiteralPath $stagingDir -Recurse -Force
            }
            New-Item -ItemType Directory -Force -Path $stagingDir | Out-Null
            tar -xzf $archivePath -C $extractDir

            $packageRoot = Join-Path $extractDir "open-interpreter"
            foreach ($binary in @("interpreter.exe", "interpreter-root-tui.exe", "interpreter-tui.exe", "interpreter-app-server.exe", "interpreter-acp.exe", "interpreter-exec.exe")) {
                Copy-Item -LiteralPath (Join-Path $packageRoot $binary) -Destination (Join-Path $stagingDir $binary)
            }

            if (Test-Path -LiteralPath $releaseDir) {
                Remove-Item -LiteralPath $releaseDir -Recurse -Force
            }
            Move-Item -LiteralPath $stagingDir -Destination $releaseDir
        }

        # Install the short `i` alias next to interpreter.exe so typing `i`
        # also launches Open Interpreter.
        $iCmdPath = Join-Path $releaseDir "i.cmd"
        Set-Content -LiteralPath $iCmdPath -Value '@"%~dp0interpreter.exe" %*' -Encoding ASCII

        New-Item -ItemType Directory -Force -Path $standaloneRoot | Out-Null
        Ensure-Junction -LinkPath $currentDir -TargetPath $releaseDir -InstallerOwnedTargetPrefix $releasesDir

        $visibleParent = Split-Path -Parent $visibleBinDir
        New-Item -ItemType Directory -Force -Path $visibleParent | Out-Null
        $oldStandaloneBackup = Move-OldStandaloneBinIfApproved -VisibleBinDir $visibleBinDir -DefaultVisibleBinDir $defaultVisibleBinDir
        try {
            Ensure-Junction -LinkPath $visibleBinDir -TargetPath $currentDir -InstallerOwnedTargetPrefix $standaloneRoot
            Test-VisibleInterpreterCommand -VisibleBinDir $visibleBinDir
        } catch {
            if ($null -ne $oldStandaloneBackup -and (Test-Path -LiteralPath $oldStandaloneBackup)) {
                if (Test-Path -LiteralPath $visibleBinDir) {
                    Remove-Item -LiteralPath $visibleBinDir -Recurse -Force
                }
                Move-Item -LiteralPath $oldStandaloneBackup -Destination $visibleBinDir
            }
            throw
        }
        if ($null -ne $oldStandaloneBackup) {
            Remove-Item -LiteralPath $oldStandaloneBackup -Recurse -Force
        }
    }
} finally {
    Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
}

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (-not (Path-Contains -PathValue $userPath -Entry $visibleBinDir)) {
    if ([string]::IsNullOrWhiteSpace($userPath)) {
        $newUserPath = $visibleBinDir
    } else {
        $newUserPath = "$visibleBinDir;$userPath"
    }

    [Environment]::SetEnvironmentVariable("Path", $newUserPath, "User")
    Write-Step "PATH updated for future PowerShell sessions."
} elseif (Path-Contains -PathValue $env:Path -Entry $visibleBinDir) {
    Write-Step "$visibleBinDir is already on PATH."
} else {
    Write-Step "PATH is already configured for future PowerShell sessions."
}

if (-not (Path-Contains -PathValue $env:Path -Entry $visibleBinDir)) {
    if ([string]::IsNullOrWhiteSpace($env:Path)) {
        $env:Path = $visibleBinDir
    } else {
        $env:Path = "$visibleBinDir;$env:Path"
    }
}

Write-Step "Current PowerShell session: interpreter"
Write-Step "Future PowerShell windows: open a new PowerShell window and run: interpreter"
Write-Host "Open Interpreter $resolvedVersion installed successfully."

$interpreterCommand = Join-Path $visibleBinDir "interpreter.exe"
if (Prompt-YesNo "Start Open Interpreter now?") {
    Write-Step "Launching Open Interpreter"
    & $interpreterCommand
}
