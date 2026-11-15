<#
.SYNOPSIS
    Windows installer for Governor v2.

.DESCRIPTION
    Installs governor.exe into %LOCALAPPDATA%\Governor\bin and adds that
    directory to the user PATH. Idempotent: re-running never duplicates the
    PATH entry and never rewrites unrelated entries. -Uninstall removes the
    binary and the PATH entry again.

.PARAMETER Uninstall
    Remove the installed binary and the PATH entry.

.PARAMETER Local
    Install from a local build (target\release\governor.exe) instead of
    downloading the latest GitHub release.

.PARAMETER LocalPath
    Explicit path to a governor.exe to install. Implies -Local.

.PARAMETER Version
    Pin a specific release tag, e.g. -Version v2.1.0. Default: latest release.

.EXAMPLE
    iwr -useb https://raw.githubusercontent.com/HDesmont/governor-v2/main/install.ps1 | iex
    Install the latest release.

.EXAMPLE
    .\install.ps1 -Local
    Install the binary you just built.

.EXAMPLE
    .\install.ps1 -Uninstall
    Remove Governor and clean the PATH.

.NOTES
    The release asset (governor-x86_64-pc-windows-msvc.zip) is produced by the
    release job in .github/workflows/ci.yml - keep the two in sync.
    When piped through "iwr | iex" the script detects the missing
    $PSCommandPath and returns instead of exiting, so your shell stays open.
#>

[CmdletBinding()]
param(
    [switch]$Uninstall,
    [switch]$Local,
    [string]$LocalPath,
    [string]$Version
)

$ErrorActionPreference = 'Stop'
# Invoke-WebRequest renders a progress bar per chunk; silencing it is ~10x faster.
$ProgressPreference = 'SilentlyContinue'

$Repo        = 'HDesmont/governor-v2'
$AssetName   = 'governor-x86_64-pc-windows-msvc.zip'   # keep in sync with .github/workflows/ci.yml
$InstallRoot = Join-Path $env:LOCALAPPDATA 'Governor'
$BinDir      = Join-Path $InstallRoot 'bin'
$ExeName     = 'governor.exe'
$ExePath     = Join-Path $BinDir $ExeName

function Write-Step([string]$msg) { Write-Host ("  ==> " + $msg) -ForegroundColor Cyan }
function Ok([string]$msg)   { Write-Host ("  [ok]   " + $msg) -ForegroundColor Green }
function Info([string]$msg) { Write-Host ("  [info] " + $msg) -ForegroundColor Gray }
function Warn([string]$msg) { Write-Host ("  [warn] " + $msg) -ForegroundColor Yellow }
function Fail([string]$msg) { Write-Host ("  [error] " + $msg) -ForegroundColor Red }

# All errors are routed here and handled once by the top-level catch:
# in script mode it exits 1, in piped mode ("iwr | iex") it just returns so
# the user's interactive shell is not killed.
function Die([string]$msg) { throw $msg }

function Test-PathEntry([string]$Entry, [string]$Target) {
    # PowerShell string -eq is case-insensitive; normalize trailing slashes so
    # "...\Governor\bin" and "...\Governor\bin\" compare equal.
    return ($Entry.Trim().TrimEnd('\') -eq $Target.Trim().TrimEnd('\'))
}

function Get-RawUserPath {
    # Read the user PATH *raw* (no %VAR% expansion). [Environment]::GetEnvironmentVariable
    # would expand REG_EXPAND_SZ values, and writing that back would rewrite
    # entries like "%USERPROFILE%\bin" as literal paths.
    try {
        $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment')
        if ($null -ne $key) {
            try {
                return [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            } finally {
                $key.Close()
            }
        }
    } catch { }
    return [string][Environment]::GetEnvironmentVariable('Path', 'User')
}

function Set-RawUserPath([string]$Value) {
    # [Environment]::SetEnvironmentVariable is the supported write API and it
    # broadcasts WM_SETTINGCHANGE, but it always writes REG_SZ. If the existing
    # value was REG_EXPAND_SZ, restore that kind afterwards so %-based entries
    # in the PATH keep working.
    $key = $null
    $kind = $null
    try { $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true) } catch { $key = $null }
    if ($null -ne $key) {
        try { $kind = $key.GetValueKind('Path') } catch { $kind = $null }
    }
    try {
        [Environment]::SetEnvironmentVariable('Path', $Value, 'User')
    } finally {
        if ($null -ne $key) {
            try {
                if ($kind -eq [Microsoft.Win32.RegistryValueKind]::ExpandString) {
                    $key.SetValue('Path', $Value, [Microsoft.Win32.RegistryValueKind]::ExpandString)
                }
            } catch { }
            $key.Close()
        }
    }
}

# Returns 'updated' | 'present' (install) or 'updated' | 'absent' (remove).
function Update-UserPath([string]$TargetDir, [switch]$Remove) {
    $raw = Get-RawUserPath
    $entries = @($raw -split ';') | Where-Object { $_.Trim() -ne '' }
    $kept = New-Object System.Collections.Generic.List[string]
    $removed = $false
    foreach ($entry in $entries) {
        $isTarget = Test-PathEntry $entry $TargetDir
        if ((-not $isTarget) -and $entry.Contains('%')) {
            # Also match the expanded form, so an entry that was stored as
            # "%LOCALAPPDATA%\Governor\bin" by another tool is still cleaned up.
            try { $isTarget = Test-PathEntry ([Environment]::ExpandEnvironmentVariables($entry)) $TargetDir } catch { $isTarget = $false }
        }
        if ($isTarget) {
            if ($Remove) { $removed = $true; continue }
            return 'present'   # already on PATH - idempotent no-op, nothing written
        }
        $kept.Add($entry)
    }
    if ($Remove) {
        if (-not $removed) { return 'absent' }
    } else {
        $kept.Add($TargetDir)
    }
    Set-RawUserPath ($kept -join ';')
    return 'updated'
}

function Add-SessionPath {
    # Make `governor` resolvable in the shell that runs the installer too.
    $parts = @($env:Path -split ';') | Where-Object { $_.Trim() -ne '' }
    foreach ($p in $parts) { if (Test-PathEntry $p $BinDir) { return } }
    $env:Path = (($parts + $BinDir) -join ';')
}

function Remove-SessionPath {
    $kept = @($env:Path -split ';') | Where-Object { $_.Trim() -ne '' -and -not (Test-PathEntry $_ $BinDir) }
    $env:Path = ($kept -join ';')
}

function Assert-NoRunningGovernor {
    # A running governor.exe locks its image file; replacing/deleting it would
    # fail with an opaque IO error, so stop early with a clear message.
    $procs = @(Get-Process -Name 'governor' -ErrorAction SilentlyContinue)
    if ($procs.Count -gt 0) {
        $ids = ($procs | ForEach-Object { $_.Id }) -join ', '
        Die ("governor.exe is currently running (PID " + $ids + ").`n" +
             "  Stop it (e.g. an active 'governor relay' or session) and re-run the installer.")
    }
}

function Get-ReleaseZipUrl {
    if ($Version) {
        $tag = $Version
        if (-not $tag.StartsWith('v')) { $tag = 'v' + $tag }
        return ("https://github.com/{0}/releases/download/{1}/{2}" -f $Repo, $tag, $AssetName)
    }
    return ("https://github.com/{0}/releases/latest/download/{1}" -f $Repo, $AssetName)
}

function Invoke-Download([string]$DestDir) {
    $url = Get-ReleaseZipUrl
    $zipPath = Join-Path $DestDir $AssetName
    Write-Step ("Downloading " + $url)
    try {
        Invoke-WebRequest -Uri $url -OutFile $zipPath -UseBasicParsing
    } catch {
        $status = $null
        try {
            if ($_.Exception.Response) { $status = [int]$_.Exception.Response.StatusCode }
        } catch { $status = $null }
        if ($status -eq 404) {
            Die ("Release asset not found (404): " + $AssetName + "`n" +
                 "  If you passed -Version, check that the tag exists.`n" +
                 "  Otherwise no release has been published yet - CI attaches this asset`n" +
                 "  on every v* tag. Or install a local build:  .\install.ps1 -Local")
        }
        Die ("Download failed: " + $_.Exception.Message + "`n" +
             "  Check your internet connection (or proxy/firewall) and re-run.")
    }

    # Best-effort integrity check when the release ships a .sha256 sidecar.
    $shaPath = Join-Path $DestDir ($AssetName + '.sha256')
    $haveSha = $false
    try {
        Invoke-WebRequest -Uri ($url + '.sha256') -OutFile $shaPath -UseBasicParsing
        $haveSha = $true
    } catch { }
    if ($haveSha) {
        $expected = ((Get-Content -LiteralPath $shaPath -TotalCount 1) -split '\s+')[0].ToLowerInvariant()
        $actual = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($expected -ne $actual) {
            Die ("SHA-256 checksum mismatch: expected " + $expected + ", got " + $actual + ".`n" +
                 "  The download is corrupted or was tampered with - re-run the installer.")
        }
        Ok "SHA-256 checksum verified"
    } else {
        Warn "Release has no checksum file - skipped verification"
    }

    Write-Step "Extracting archive"
    $extractDir = Join-Path $DestDir 'extract'
    try {
        Expand-Archive -LiteralPath $zipPath -DestinationPath $extractDir -Force
    } catch {
        Die ("Could not extract the archive: " + $_.Exception.Message)
    }
    $found = @(Get-ChildItem -LiteralPath $extractDir -Recurse -Filter $ExeName -File)
    if ($found.Count -eq 0) {
        Die ("Downloaded archive does not contain " + $ExeName + " - unexpected package layout.")
    }
    return $found[0].FullName
}

function Invoke-Install {
    Write-Host ""
    Write-Host ("  Installing Governor -> " + $BinDir) -ForegroundColor White
    Write-Host ""
    if (-not $env:LOCALAPPDATA) {
        Die "LOCALAPPDATA is not set - this installer targets Windows user profiles."
    }

    $sourceExe = $null
    $tmpDir = $null
    try {
        if ($Local -or $LocalPath) {
            $candidate = $LocalPath
            if (-not $candidate) {
                # Piped via "iex" there is no script location; fall back to CWD.
                $base = $PSScriptRoot
                if (-not $base) { $base = (Get-Location).Path }
                $candidate = Join-Path (Join-Path $base 'target\release') $ExeName
            }
            if (-not (Test-Path -LiteralPath $candidate)) {
                Die ("Local binary not found: " + $candidate + "`n" +
                     "  Build it first:   cargo build --release`n" +
                     "  Or drop -Local/-LocalPath to download the latest release instead.")
            }
            $sourceExe = (Resolve-Path -LiteralPath $candidate).Path
            Write-Step ("Source: local build " + $sourceExe)
        } else {
            $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ("governor-install-" + [Guid]::NewGuid().ToString('N').Substring(0, 8))
            New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null
            $sourceExe = Invoke-Download $tmpDir
        }

        Assert-NoRunningGovernor
        Write-Step ("Creating " + $BinDir)
        New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
        try {
            Copy-Item -LiteralPath $sourceExe -Destination $ExePath -Force
        } catch {
            Die ("Could not write " + $ExePath + ": " + $_.Exception.Message + "`n" +
                 "  Check permissions, close programs using the file, then re-run.")
        }
        Ok ("Binary in place: " + $ExePath)

        switch (Update-UserPath -TargetDir $BinDir) {
            'updated' { Ok ("Added " + $BinDir + " to the user PATH") }
            'present' { Info ("User PATH already contains " + $BinDir + " - nothing to change") }
        }
        Add-SessionPath
    } finally {
        if ($tmpDir -and (Test-Path -LiteralPath $tmpDir)) {
            Remove-Item -LiteralPath $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    $versionLine = ''
    try { $versionLine = ((& $ExePath --version) 2>$null) -join ' ' } catch { }
    if (-not $versionLine) { $versionLine = 'installed (version check skipped)' }

    Write-Host ""
    Write-Host "  ============================================================" -ForegroundColor Green
    Write-Host ("   Governor installed:  " + $versionLine) -ForegroundColor Green
    Write-Host ("   Location:            " + $ExePath) -ForegroundColor Green
    Write-Host "   PATH:                user PATH updated (idempotent)" -ForegroundColor Green
    Write-Host ""
    Write-Host "   Open a NEW terminal and run:   governor --version" -ForegroundColor White
    Write-Host "   (already-open terminals keep their old PATH)" -ForegroundColor Gray
    Write-Host "  ============================================================" -ForegroundColor Green
}

function Invoke-Uninstall {
    Write-Host ""
    Write-Host ("  Uninstalling Governor from " + $BinDir) -ForegroundColor White
    Write-Host ""
    Assert-NoRunningGovernor
    if (Test-Path -LiteralPath $ExePath) {
        try {
            Remove-Item -LiteralPath $ExePath -Force
            Ok ("Removed " + $ExePath)
        } catch {
            Die ("Could not delete " + $ExePath + ": " + $_.Exception.Message)
        }
    } else {
        Info ("Binary already absent: " + $ExePath)
    }
    foreach ($dir in @($BinDir, $InstallRoot)) {
        if ((Test-Path -LiteralPath $dir) -and -not (@(Get-ChildItem -LiteralPath $dir -Force))) {
            try {
                Remove-Item -LiteralPath $dir -Force
                Ok ("Removed empty directory " + $dir)
            } catch { }
        }
    }
    switch (Update-UserPath -TargetDir $BinDir -Remove) {
        'updated' { Ok ("Removed " + $BinDir + " from the user PATH") }
        'absent'  { Info ("User PATH has no entry for " + $BinDir + " - nothing to clean") }
    }
    Remove-SessionPath
    Write-Host ""
    Ok "Governor uninstalled."
    Info "Restart open terminals so they drop the removed PATH entry."
}

try {
    if ($env:OS -ne 'Windows_NT' -and -not $IsWindows) {
        Die "This installer is Windows-only. On Linux/macOS: cargo build --release"
    }
    if ($PSVersionTable.PSVersion.Major -lt 5) {
        Die ("PowerShell 5.1 or newer is required, found " + $PSVersionTable.PSVersion.ToString() + ".")
    }
    # Windows PowerShell 5.1 may negotiate TLS 1.0/1.1; GitHub requires TLS 1.2+.
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    } catch { }

    if ($Uninstall) { Invoke-Uninstall } else { Invoke-Install }
} catch {
    Fail $_.Exception.Message
    # Piped via "iwr | iex": return instead of exit so the user's shell stays open.
    if ([string]::IsNullOrEmpty($PSCommandPath)) { return }
    exit 1
}
if ([string]::IsNullOrEmpty($PSCommandPath)) { return }
exit 0
