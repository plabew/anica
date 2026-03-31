Param(
    [ValidateSet("system", "local-lgpl")]
    [string]$Mode = "system",
    [switch]$InstallGStreamer,
    [switch]$Yes,
    [string]$ToolsHome = "$env:LOCALAPPDATA\Anica\tools",
    [string]$FfmpegVersion = "8.0.1"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ToolsHome = [System.IO.Path]::GetFullPath($ToolsHome)

function Log([string]$Message) {
    Write-Host "[setup] $Message"
}

function Warn([string]$Message) {
    Write-Warning "[setup] $Message"
}

function Has-Command([string]$Name) {
    return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

function Resolve-CommandExecutable([string]$Name) {
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -eq $cmd) {
        return $null
    }
    if ($cmd.CommandType -ne "Application") {
        return $null
    }
    return $cmd.Source
}

function Ensure-ToolsHome {
    if (-not (Test-Path -LiteralPath $ToolsHome)) {
        New-Item -ItemType Directory -Force -Path $ToolsHome | Out-Null
    }
}

function Invoke-WingetInstall([string]$PackageId) {
    if (-not (Has-Command "winget")) {
        return $false
    }

    $args = @(
        "install",
        "--id", $PackageId,
        "-e",
        "--accept-package-agreements",
        "--accept-source-agreements"
    )
    if ($Yes) {
        $args += "--silent"
    }

    Log "Trying winget package: $PackageId"
    & winget @args | Out-Null
    return ($LASTEXITCODE -eq 0)
}

function Ensure-SystemFfmpeg {
    if (Has-Command "ffmpeg" -and Has-Command "ffprobe") {
        Log "System ffmpeg/ffprobe already present."
        return
    }

    if (-not (Has-Command "winget")) {
        Warn "winget not found. Install FFmpeg manually and add to PATH."
        Warn "Suggested source: https://www.gyan.dev/ffmpeg/builds/"
        return
    }

    # Try common package IDs used on Windows.
    $candidates = @(
        "Gyan.FFmpeg",
        "BtbN.FFmpeg"
    )
    foreach ($pkg in $candidates) {
        if (Invoke-WingetInstall $pkg) {
            break
        }
    }
}

function Ensure-GStreamer {
    if (Has-Command "gst-launch-1.0") {
        Log "Found GStreamer CLI: $(Resolve-CommandExecutable "gst-launch-1.0")"
        return
    }

    Warn "GStreamer CLI not found."
    if (-not $InstallGStreamer) {
        Warn "Install manually from https://gstreamer.freedesktop.org/download/ (MSVC x86_64 runtime + development)."
        return
    }

    if (-not (Has-Command "winget")) {
        Warn "winget not found. Install GStreamer manually."
        return
    }

    $gstCandidates = @(
        "GStreamer.GStreamer"
    )
    $installed = $false
    foreach ($pkg in $gstCandidates) {
        if (Invoke-WingetInstall $pkg) {
            $installed = $true
            break
        }
    }

    if (-not $installed) {
        Warn "Auto-install for GStreamer failed. Install manually from https://gstreamer.freedesktop.org/download/."
    }
}

function Get-FfmpegConfigLine([string]$FfmpegExe) {
    $version = & $FfmpegExe -hide_banner -version 2>$null
    if ($LASTEXITCODE -ne 0) {
        return $null
    }
    foreach ($line in $version) {
        if ($line -like "configuration:*") {
            return $line.ToString().ToLowerInvariant()
        }
    }
    return $null
}

function Test-FfmpegIsLgplOnly([string]$FfmpegExe) {
    $cfg = Get-FfmpegConfigLine $FfmpegExe
    if ([string]::IsNullOrWhiteSpace($cfg)) {
        Warn "Unable to read ffmpeg build configuration; treating as not verified LGPL-only."
        return $false
    }
    if ($cfg.Contains("--enable-gpl") -or $cfg.Contains("--enable-nonfree")) {
        return $false
    }
    return $true
}

function Remove-IfExists([string]$Path) {
    if (Test-Path -LiteralPath $Path) {
        Remove-Item -LiteralPath $Path -Force -Recurse
    }
}

function Copy-TreePhysical([string]$SourceDir, [string]$TargetDir) {
    if (-not (Test-Path -LiteralPath $SourceDir)) {
        return $false
    }
    $parent = Split-Path -Parent $TargetDir
    if (-not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    Remove-IfExists $TargetDir
    New-Item -ItemType Directory -Path $TargetDir -Force | Out-Null
    Copy-Item -Path (Join-Path $SourceDir "*") -Destination $TargetDir -Recurse -Force
    return $true
}

function Sync-FfmpegRuntime {
    $ffmpegExe = Resolve-CommandExecutable "ffmpeg"
    $ffprobeExe = Resolve-CommandExecutable "ffprobe"
    if ([string]::IsNullOrWhiteSpace($ffmpegExe) -or [string]::IsNullOrWhiteSpace($ffprobeExe)) {
        Warn "Cannot sync FFmpeg runtime: ffmpeg/ffprobe not found in PATH."
        return
    }

    $ffmpegBinDir = Split-Path -Parent $ffmpegExe
    $runtimeRoot = Join-Path $ToolsHome "ffmpeg"
    $runtimeBin = Join-Path $runtimeRoot "bin"
    $runtimeLib = Join-Path $runtimeRoot "lib"
    New-Item -ItemType Directory -Path $runtimeRoot -Force | Out-Null

    if (-not (Copy-TreePhysical $ffmpegBinDir $runtimeBin)) {
        throw "Failed to sync FFmpeg bin directory into runtime."
    }

    $ffmpegPrefix = Split-Path -Parent $ffmpegBinDir
    $sourceLib = Join-Path $ffmpegPrefix "lib"
    if (Test-Path -LiteralPath $sourceLib) {
        [void](Copy-TreePhysical $sourceLib $runtimeLib)
    }

    Log "FFmpeg runtime synced: $runtimeBin"
}

function Sync-GStreamerRuntime {
    $gstExe = Resolve-CommandExecutable "gst-launch-1.0"
    if ([string]::IsNullOrWhiteSpace($gstExe)) {
        Warn "Cannot sync GStreamer runtime: gst-launch-1.0 not found."
        return
    }

    $gstBinDir = Split-Path -Parent $gstExe
    $gstPrefix = Split-Path -Parent $gstBinDir
    $gstLibDir = Join-Path $gstPrefix "lib"
    if (-not (Test-Path -LiteralPath $gstLibDir)) {
        $gstLibDir = $null
    }

    $runtimeRoot = Join-Path $ToolsHome "gstreamer"
    $runtimeBin = Join-Path $runtimeRoot "bin"
    $runtimeLib = Join-Path $runtimeRoot "lib"
    New-Item -ItemType Directory -Path $runtimeRoot -Force | Out-Null

    if (-not (Copy-TreePhysical $gstBinDir $runtimeBin)) {
        throw "Failed to sync GStreamer bin directory into runtime."
    }
    if ($null -ne $gstLibDir) {
        [void](Copy-TreePhysical $gstLibDir $runtimeLib)
    }

    $pluginRoot = Join-Path $runtimeLib "gstreamer-1.0"
    if (Test-Path -LiteralPath $pluginRoot) {
        $dropPatterns = @(
            "libgst*bad*",
            "libgst*ugly*",
            "libgstx264*",
            "libgstx265*",
            "libgstfdkaac*",
            "libgstfaac*",
            "libgstlame*",
            "libgstpython*",
            "libgstgtk*"
        )
        foreach ($pattern in $dropPatterns) {
            Get-ChildItem -Path $pluginRoot -Filter $pattern -File -ErrorAction SilentlyContinue |
                Remove-Item -Force -ErrorAction SilentlyContinue
        }
    }

    Log "GStreamer runtime synced: $runtimeBin"
}

function Write-EnvHints {
    $ffmpegHint = Join-Path $ToolsHome "ffmpeg\bin\ffmpeg.exe"
    Log "Runtime root:"
    Write-Host "  $ToolsHome"
    Log "Optional env overrides:"
    Write-Host "  setx ANICA_TOOLS_HOME `"$ToolsHome`""
    Write-Host "  setx ANICA_FFMPEG_PATH `"$ffmpegHint`""
}

Log "Anica media tools bootstrap (Windows)"
Log "Mode: $Mode"
Log "Tools home: $ToolsHome"
Log "FFmpeg target version: $FfmpegVersion"

Ensure-ToolsHome
Ensure-GStreamer

if ($Mode -eq "system") {
    Ensure-SystemFfmpeg
    Sync-FfmpegRuntime
    Sync-GStreamerRuntime
} else {
    # local-lgpl on Windows uses a strict verifier:
    # reject ffmpeg binaries built with --enable-gpl / --enable-nonfree.
    Ensure-SystemFfmpeg

    $ffmpegExe = Resolve-CommandExecutable "ffmpeg"
    if ([string]::IsNullOrWhiteSpace($ffmpegExe)) {
        throw "local-lgpl mode requires ffmpeg in PATH before runtime sync."
    }
    if (-not (Test-FfmpegIsLgplOnly $ffmpegExe)) {
        throw "Detected ffmpeg build is not verified LGPL-only (or config not readable). Provide a vetted LGPL-only ffmpeg build and retry."
    }

    Sync-FfmpegRuntime
    Sync-GStreamerRuntime
}

if (Has-Command "ffmpeg") {
    Log "ffmpeg detected:"
    ffmpeg -version | Select-Object -First 3
}

Write-EnvHints
Log "Done."
