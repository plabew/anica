Param(
    [switch]$Yes,
    [switch]$SyncOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $PSCommandPath
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")
$manifestPath = Join-Path $repoRoot "tools" "media_tools_manifest.json"
$runtimeDir = Join-Path $repoRoot "tools" "runtime"

function Log([string]$Message) {
    Write-Host "[setup] $Message"
}

function Warn([string]$Message) {
    Write-Warning "[setup] $Message"
}

function Has-Command([string]$Name) {
    return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

# Read a runtime version from the manifest.
function Read-ManifestVersion([string]$Tool) {
    if (-not (Test-Path $manifestPath)) {
        return $null
    }
    try {
        $manifest = Get-Content $manifestPath | ConvertFrom-Json
        return $manifest.common.$Tool.version
    } catch {
        return $null
    }
}

# Read the platform download URL from the manifest.
function Read-ManifestDownloadUrl([string]$Os) {
    if (-not (Test-Path $manifestPath)) {
        return $null
    }
    try {
        $manifest = Get-Content $manifestPath | ConvertFrom-Json
        $url = $manifest.platforms.$Os.download_url
        $baseUrl = $manifest.release_base_url
        if ($url -and $baseUrl) {
            return $url -replace "{release_base_url}", $baseUrl
        }
        return $url
    } catch {
        return $null
    }
}

# Copy a directory tree from src to dst.
function Copy-RuntimeTree([string]$Source, [string]$Destination) {
    $parent = Split-Path -Parent $Destination
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    if (Test-Path $Destination) {
        Remove-Item -Recurse -Force -Path $Destination
    }
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    Copy-Item -Recurse -Force -Path (Join-Path $Source "*") -Destination $Destination
}

# Check if runtime is complete at a given root.
function Runtime-Complete([string]$Root) {
    $ffmpeg = Join-Path $Root "ffmpeg\bin\ffmpeg.exe"
    $gst = Join-Path $Root "gstreamer\bin\gst-launch-1.0.exe"
    return (Test-Path $ffmpeg) -and (Test-Path $gst)
}

# Find the runtime root inside the extracted directory.
# The tar.gz may have an extra top-level folder (e.g. anica_runtime_windows_20260610).
function Find-RuntimeRootInStaging([string]$Staging) {
    # If the staging root already contains ffmpeg/gstreamer, use it directly.
    if ((Test-Path (Join-Path $Staging "ffmpeg")) -and (Test-Path (Join-Path $Staging "gstreamer"))) {
        return $Staging
    }
    # Otherwise look one level deeper for the actual runtime folder.
    $subdirs = Get-ChildItem -Path $Staging -Directory -Recurse -Depth 1 |
        Where-Object { $_.Name -eq "ffmpeg" -or $_.Name -eq "gstreamer" } |
        Select-Object -First 1
    if ($subdirs) {
        return Split-Path -Parent $subdirs.FullName
    }
    return $null
}

# Download and extract the runtime archive for the current platform.
function Download-Runtime {
    $os = "windows"
    $platformDir = Join-Path $runtimeDir $os
    $currentDir = Join-Path (Join-Path $runtimeDir "current") $os

    $ffmpegVersion = Read-ManifestVersion "ffmpeg"
    $gstVersion = Read-ManifestVersion "gstreamer"
    if (-not $ffmpegVersion) {
        Warn "No FFmpeg version in manifest."
        return
    }
    if (-not $gstVersion) {
        Warn "No GStreamer version in manifest."
        return
    }

    # Check if called with -SyncOnly (just sync versioned to current, no download)
    if ($SyncOnly) {
        $ffmpegSrc = Join-Path $platformDir "ffmpeg\$ffmpegVersion"
        $gstSrc = Join-Path $platformDir "gstreamer\$gstVersion"
        if ((Test-Path $ffmpegSrc) -and (Test-Path $gstSrc)) {
            Log "Syncing versioned runtime to current (sync-only)..."
            Copy-RuntimeTree $ffmpegSrc (Join-Path $currentDir "ffmpeg")
            Copy-RuntimeTree $gstSrc (Join-Path $currentDir "gstreamer")
            Log "Runtime ready: ${currentDir}"
            return
        } else {
            Warn "Versioned runtime not found for sync-only."
            return
        }
    }

    # Already present?
    if (Runtime-Complete $currentDir) {
        Log "Runtime already present at ${currentDir}"
        return
    }

    # If versioned folders exist, just sync to current.
    $ffmpegVersioned = Join-Path $platformDir "ffmpeg\$ffmpegVersion"
    $gstVersioned = Join-Path $platformDir "gstreamer\$gstVersion"
    if ((Test-Path $ffmpegVersioned) -and (Test-Path $gstVersioned)) {
        Log "Syncing versioned runtime to current..."
        Copy-RuntimeTree $ffmpegVersioned (Join-Path $currentDir "ffmpeg")
        Copy-RuntimeTree $gstVersioned (Join-Path $currentDir "gstreamer")
        Log "Runtime ready: ${currentDir}"
        return
    }

    # Download from manifest.
    $url = Read-ManifestDownloadUrl $os
    if (-not $url) {
        Warn "No download URL for ${os}."
        return
    }

    $archivePath = Join-Path $runtimeDir "anica-runtime-${os}.tar.gz"

    Log "Downloading Anica runtime..."
    Log "URL: ${url}"
    New-Item -ItemType Directory -Force -Path $runtimeDir | Out-Null

    if (-not (Has-Command "curl.exe")) {
        Warn "curl.exe not found. Cannot download runtime."
        return
    }

    try {
        curl.exe -fL --progress-bar $url -o $archivePath
    } catch {
        Warn "Download failed: ${url}"
        return
    }

    # Extract to a temp directory.
    $stagingDir = Join-Path $runtimeDir ".extract-${os}-$([guid]::NewGuid().ToString('N'))"
    Log "Extracting to ${stagingDir}..."
    New-Item -ItemType Directory -Force -Path $stagingDir | Out-Null

    # Windows 10/11 has built-in tar; use it to extract tar.gz.
    tar -xzf $archivePath -C $stagingDir

    # Find the actual runtime root (may be nested inside a top-level folder).
    $runtimeRoot = Find-RuntimeRootInStaging $stagingDir
    if (-not $runtimeRoot) {
        Warn "Could not find ffmpeg/gstreamer inside extracted archive."
        Remove-Item -Recurse -Force -Path $stagingDir
        Remove-Item -Path $archivePath -Force
        return
    }

    Log "Found runtime root: ${runtimeRoot}"

    # Check if the runtime root already contains versioned subfolders (e.g. ffmpeg/8.0.1).
    # If so, copy the entire runtime root directly to the platform dir.
    # If not, copy the individual tool folders into versioned paths.
    New-Item -ItemType Directory -Force -Path $platformDir | Out-Null
    New-Item -ItemType Directory -Force -Path $currentDir | Out-Null

    $ffmpegInStaging = Join-Path $runtimeRoot "ffmpeg\$ffmpegVersion"
    $gstInStaging = Join-Path $runtimeRoot "gstreamer\$gstVersion"

    if ((Test-Path $ffmpegInStaging) -and (Test-Path $gstInStaging)) {
        Log "Runtime already versioned. Copying directly to ${platformDir}..."
        Copy-RuntimeTree $runtimeRoot $platformDir
    } else {
        Log "Copying ffmpeg to versioned path..."
        Copy-RuntimeTree (Join-Path $runtimeRoot "ffmpeg") $ffmpegVersioned
        Log "Copying gstreamer to versioned path..."
        Copy-RuntimeTree (Join-Path $runtimeRoot "gstreamer") $gstVersioned
    }

    # Sync to current (unversioned) for app consumption.
    Log "Syncing to current/..."
    Copy-RuntimeTree $ffmpegVersioned (Join-Path $currentDir "ffmpeg")
    Copy-RuntimeTree $gstVersioned (Join-Path $currentDir "gstreamer")

    # Clean up.
    Remove-Item -Path $archivePath -Force
    Remove-Item -Recurse -Force -Path $stagingDir

    Log "Runtime ready: ${currentDir}"
    Log "Versioned: ${ffmpegVersioned}, ${gstVersioned}"
}

Log "Anica runtime bootstrap (Windows)"
Log "Manifest: ${manifestPath}"

Download-Runtime

Log "Done."
